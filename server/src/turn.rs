use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::cities::{City, CityOwner};
use shared::events::StartGame;
use shared::events::*;
use shared::unit_definition::{UnitRegistry, is_within_move_range};
use shared::unit_definition::{available_verbs, is_within_attack_range};
use shared::units::*;
use shared::{components::*, hex::HexPosition};

use crate::GRID_RADIUS;
use crate::cities::spawn_city_at_tile;
use crate::players::PlayerMap;

const MIN_CITY_CENTER_DISTANCE: i32 = 4;

/// Represents whether player is still making moves or has finished his turn
#[derive(PartialEq, Eq)]
pub enum PlayerTurnState {
    InProgress,
    Finished,
}

/// Stores information about players
/// TODO: add some methods to automatically update finished_cnt
#[derive(Resource, Default)]
pub struct PlayerState {
    pub turn: HashMap<Entity, PlayerTurnState>,
    pub finished_cnt: i32,
}

pub fn update_turn_phase(players: Query<(), With<Player>>, mut turn_state: Query<&mut TurnState>) {
    let count = players.iter().count();
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };

    match state.phase {
        TurnPhase::Lobby => {
            // No auto-advance; host must send StartGame.
        }
        TurnPhase::WaitingForPlayers => {
            if count == 0 {
                // All players gone — reset for the next session.
                state.phase = TurnPhase::Lobby;
                state.turn_number = 0;
                println!("All players disconnected, server reset to Lobby");
            } else if count >= 2 {
                state.phase = TurnPhase::Accepting;
                println!(
                    "Enough players ({count}), resuming turn {}",
                    state.turn_number
                );
            }
        }
        TurnPhase::Accepting => {
            if count == 0 {
                // Fast-path: both players disconnected in the same frame.
                state.phase = TurnPhase::Lobby;
                state.turn_number = 0;
                println!("All players disconnected, server reset to Lobby");
            } else if count < 2 {
                state.phase = TurnPhase::WaitingForPlayers;
                println!("Not enough players ({count}), waiting...");
            }
        }
    }
}

pub fn handle_start_game(
    trigger: On<FromClient<StartGame>>,
    player_map: Res<PlayerMap>,
    hosts: Query<(), With<Host>>,
    players: Query<(), With<Player>>,
    mut turn_state: Query<&mut TurnState>,
) {
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };
    if state.phase != TurnPhase::Lobby {
        return;
    }

    let client_entity = match trigger.client_id {
        ClientId::Client(e) => e,
        ClientId::Server => return,
    };
    let Some(&player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };

    if !hosts.contains(player_entity) {
        println!("Rejected StartGame: sender is not the host");
        return;
    }
    let count = players.iter().count();
    if count < 2 {
        println!("Rejected StartGame: need >= 2 players, have {count}");
        return;
    }

    state.phase = TurnPhase::Accepting;
    println!(
        "Game started by host. Accepting moves for turn {}",
        state.turn_number
    );
}

pub fn handle_finish_turn(
    trigger: On<FromClient<FinishTurn>>,
    mut player_state: ResMut<PlayerState>,
) {
    let client_entity = match trigger.client_id {
        ClientId::Client(entity) => entity,
        ClientId::Server => return,
    };
    let prev_state = player_state
        .turn
        .insert(client_entity, PlayerTurnState::Finished);
    if prev_state.is_none_or(|state| state == PlayerTurnState::InProgress) {
        player_state.finished_cnt += 1;
    }
    let cnt = player_state.finished_cnt;
    println!("Received finish turn from player {client_entity}. Finished cnt {cnt}");
}

// replace the unit's queued action marker in one shot — single-action invariant
fn queue_marker<M: Component>(commands: &mut Commands, entity: Entity, marker: M) {
    commands
        .entity(entity)
        .remove::<MoveTo>()
        .remove::<AttackTarget>()
        .remove::<Fortifying>()
        .remove::<BuildProject>()
        .remove::<Skipping>()
        .insert(marker);
}

#[allow(clippy::too_many_arguments)]
pub fn handle_unit_action(
    trigger: On<FromClient<UnitActionEvent>>,
    mut commands: Commands,
    player_map: Res<PlayerMap>,
    units: Query<(&HexPosition, &Owner, &Unit)>,
    enemy_units: Query<(&HexPosition, &Owner), With<Unit>>,
    cities: Query<(&HexPosition, &CityOwner), With<City>>,
    queued_moves: Query<(Entity, &MoveTo, &Owner), With<Unit>>, // for same-turn collision detection
    turn_state: Query<&TurnState>,
    registry: Res<UnitRegistry>,
) {
    // common-path validation
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let client_entity = match trigger.client_id {
        ClientId::Client(e) => e,
        ClientId::Server => return,
    };
    let Some(player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };

    let entity = trigger.message.unit;
    let Ok((pos, owner, unit)) = units.get(entity) else {
        return;
    };
    if owner.0 != *player_entity {
        return;
    }

    let Some(def) = registry.get(&unit.type_id) else {
        println!("Rejected action: unknown unit type {:?}", unit.type_id);
        return;
    };
    let verb = trigger.message.action.verb();
    if !available_verbs(def).contains(&verb) {
        println!(
            "Rejected action: verb {:?} not available for unit type",
            verb
        );
        return;
    }

    match &trigger.message.action {
        UnitAction::Move { target } => {
            if !is_within_move_range(pos, target, def.move_budget) {
                println!("Rejected move: out of range");
                return;
            }
            if !target.in_bounds(GRID_RADIUS) {
                println!("Rejected move: out of bounds");
                return;
            }
            // Reject if a friendly is currently standing on the target tile.
            if units
                .iter()
                .any(|(p, o, _)| p == target && o.0 == *player_entity)
            {
                println!("Rejected move: friendly already on tile");
                return;
            }
            if let Some((_, city_owner)) = cities.iter().find(|(p, _)| *p == target)
                && city_owner.entity != *player_entity
                && def.attack_range != 1
            {
                println!("Rejected move: only melee units can attack cities by moving");
                return;
            }
            // Reject if another friendly already queued a Move to the same tile this turn.
            // Skip the issuing unit's own marker — re-submitting the same target on the
            // same unit must succeed (it's a no-op replace handled by queue_marker).
            if queued_moves
                .iter()
                .any(|(e, mv, o)| e != entity && mv.pos == *target && o.0 == *player_entity)
            {
                println!("Rejected move: tile already targeted by another friendly");
                return;
            }
            println!("Queued Move: unit {entity} (player {player_entity}) {pos:?} -> {target:?}");
            queue_marker(&mut commands, entity, MoveTo { pos: *target });
        }
        UnitAction::Attack { target } => {
            if !is_within_attack_range(pos, target, def.attack_range) {
                println!("Rejected attack: out of range");
                return;
            }
            let enemy_here = enemy_units
                .iter()
                .any(|(p, o)| p == target && o.0 != *player_entity)
                || cities
                    .iter()
                    .any(|(p, owner)| p == target && owner.entity != *player_entity);
            if !enemy_here {
                println!("Rejected attack: no enemy at target");
                return;
            }
            println!(
                "Queued Attack: unit {entity} (player {player_entity}) at {pos:?} -> {target:?}"
            );
            queue_marker(&mut commands, entity, AttackTarget { pos: *target });
        }
        UnitAction::Fortify => {
            println!("Queued Fortify: unit {entity} (player {player_entity})");
            queue_marker(&mut commands, entity, Fortifying);
        }
        UnitAction::Skip => {
            println!("Queued Skip: unit {entity} (player {player_entity})");
            queue_marker(&mut commands, entity, Skipping);
        }
        UnitAction::Build { project } => {
            if !def.build_targets.contains(project) {
                println!("Rejected build: project {project:?} not buildable");
                return;
            }
            println!("Queued Build: unit {entity} (player {player_entity}) project {project:?}");
            queue_marker(
                &mut commands,
                entity,
                BuildProject {
                    name: project.clone(),
                },
            );
        }
    }
}

pub fn resolve_fortify(units: Query<Entity, With<Fortifying>>, mut commands: Commands) {
    // stub: persistent Fortified state added by combat-resolution spec
    for entity in &units {
        println!("(stub) fortify {entity:?}");
        commands.entity(entity).remove::<Fortifying>();
    }
}

pub fn resolve_skip(units: Query<Entity, With<Skipping>>, mut commands: Commands) {
    // stub: passive heal lands in a separate spec
    for entity in &units {
        println!("(stub) skip {entity:?}");
        commands.entity(entity).remove::<Skipping>();
    }
}

pub fn resolve_builds(
    units: Query<(Entity, &HexPosition, &Owner, &ColorIndex, &BuildProject)>,
    cities: Query<&HexPosition, With<City>>,
    mut commands: Commands,
) {
    // stub: project advancement lands in city/economy spec
    let mut city_centers = cities.iter().copied().collect::<Vec<_>>();
    for (entity, pos, owner, color, build) in &units {
        if build.name == "city" {
            let too_close = city_centers
                .iter()
                .any(|city_pos| pos.distance(city_pos) < MIN_CITY_CENTER_DISTANCE);
            if too_close {
                println!(
                    "Rejected city settlement by {entity:?}: city center must be at least {MIN_CITY_CENTER_DISTANCE} tiles from existing cities"
                );
                commands.entity(entity).remove::<BuildProject>();
                continue;
            }

            println!("Settling city by {entity:?}");
            spawn_city_at_tile(&mut commands, *pos, owner.0, color.0);
            city_centers.push(*pos);
            commands.entity(entity).despawn();
        } else {
            println!("(stub) build {} on {entity:?}", build.name);
            commands.entity(entity).remove::<BuildProject>();
        }
    }
}

pub fn advance_turn(mut turn_state: Query<&mut TurnState>, mut player_state: ResMut<PlayerState>) {
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };
    state.turn_number += 1;
    player_state.finished_cnt = 0;
    for val in player_state.turn.values_mut() {
        *val = PlayerTurnState::InProgress;
    }
    println!("Turn resolved! Now on turn {}", state.turn_number);
}

// run condition: the resolution window — phase = Accepting AND every
// connected player has hit Finish Turn (>= 2 players to start a turn at all)
pub fn turn_is_resolving(
    turn_state: Query<&TurnState>,
    player_state: Res<PlayerState>,
    players: Query<(), With<Player>>,
) -> bool {
    let Ok(state) = turn_state.single() else {
        return false;
    };
    if state.phase != TurnPhase::Accepting {
        return false;
    }
    let count = players.iter().count() as i32;
    count >= 2 && player_state.finished_cnt >= count
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bevy::app::ScheduleRunnerPlugin;
    use bevy::state::app::StatesPlugin;
    use bevy_replicon::prelude::*;
    use shared::cities::{City, CityOwner};
    use shared::components::*;
    use shared::events::*;
    use shared::unit_definition::*;
    use shared::units::*;

    use super::*;
    use crate::players::PlayerMap;

    /// Regression test: a rejected action must NOT clear a previously queued valid marker.
    ///
    /// Scenario: unit already has `MoveTo` queued. Player submits an invalid Attack
    /// (no enemy at the target hex). The `MoveTo` must survive unchanged.
    #[test]
    fn test_rejected_action_preserves_prior_marker() {
        // Minimal Bevy app — no SharedPlugin (avoids assets/ file I/O startup).
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));

        // Build a warrior-like registry entry directly, without touching the filesystem.
        let warrior_type = UnitTypeId(0);
        let warrior_def = UnitDefinition {
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 3,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry
            .name_to_id
            .insert("warrior".to_string(), warrior_type);
        registry.definitions.insert(warrior_type, warrior_def);

        app.insert_resource(registry);
        app.init_resource::<PlayerState>();
        app.init_resource::<PlayerMap>();
        app.add_observer(handle_unit_action);

        // Flush the startup so RepliconPlugins initialises its state.
        app.update();

        // Spawn the game world: TurnState in Accepting, two players, one owned unit.
        let (client_entity, player_entity, unit_entity) = {
            let world = app.world_mut();

            // Turn state in Accepting phase.
            world.spawn(TurnState {
                phase: TurnPhase::Accepting,
                turn_number: 1,
            });

            // Owning player entity.
            let player_entity = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                    slot_index: 0,
                })
                .id();

            // "Client" entity — just a marker entity replicon would have created.
            let client_entity = world.spawn_empty().id();

            // Wire the client → player mapping.
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client_entity, player_entity);

            // Friendly unit at (0, 0) with a MoveTo already queued.
            let unit_entity = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(0, 0),
                    Owner(player_entity),
                    MoveTo {
                        pos: HexPosition::new(1, 0),
                    },
                ))
                .id();

            (client_entity, player_entity, unit_entity)
        };

        // Sanity: MoveTo is present before the rejected action.
        assert!(
            app.world().get::<MoveTo>(unit_entity).is_some(),
            "precondition: MoveTo should be present before the test"
        );

        // Trigger an invalid Attack: target hex (3, 3) has no enemy on it.
        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client_entity),
            message: UnitActionEvent {
                unit: unit_entity,
                action: UnitAction::Attack {
                    target: HexPosition::new(3, 3),
                },
            },
        });

        // Flush deferred commands from the observer.
        app.world_mut().flush();

        // The prior MoveTo must still be present — the rejected attack must not clear it.
        assert!(
            app.world().get::<MoveTo>(unit_entity).is_some(),
            "MoveTo was wrongly cleared by a rejected Attack action"
        );
        // And no AttackTarget should have been inserted.
        assert!(
            app.world().get::<AttackTarget>(unit_entity).is_none(),
            "AttackTarget was wrongly inserted for a rejected Attack"
        );

        let _ = (player_entity,); // suppress unused-variable warning
    }

    #[test]
    fn test_resolve_fortify_removes_marker() {
        use bevy::prelude::*;
        use shared::hex::HexPosition;
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, resolve_fortify);
        let entity = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(0, 0),
                Fortifying,
            ))
            .id();
        app.update();
        assert!(app.world().get::<Fortifying>(entity).is_none());
    }

    #[test]
    fn test_resolve_skip_removes_marker() {
        use bevy::prelude::*;
        use shared::hex::HexPosition;
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, resolve_skip);
        let entity = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(0, 0),
                Skipping,
            ))
            .id();
        app.update();
        assert!(app.world().get::<Skipping>(entity).is_none());
    }

    #[test]
    fn test_resolve_builds_removes_settler() {
        use bevy::prelude::*;
        use shared::hex::HexPosition;
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, resolve_builds);
        let player = app
            .world_mut()
            .spawn(Player {
                color_index: 0,
                gold: 0,
                slot_index: 0,
            })
            .id();
        let entity = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(0, 0),
                BuildProject {
                    name: "city".into(),
                },
                Owner(player),
                ColorIndex(0),
            ))
            .id();
        app.update();
        assert!(!app.world().entities().contains(entity));
    }

    #[test]
    fn test_resolve_builds_rejects_city_too_close_to_existing_city() {
        use bevy::prelude::*;
        use shared::cities::City;
        use shared::hex::HexPosition;
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, resolve_builds);
        let player = app
            .world_mut()
            .spawn(Player {
                color_index: 0,
                gold: 0,
                slot_index: 0,
            })
            .id();
        app.world_mut().spawn((City, HexPosition::new(0, 0)));
        let entity = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(3, 0),
                BuildProject {
                    name: "city".into(),
                },
                Owner(player),
                ColorIndex(0),
            ))
            .id();

        app.update();

        assert!(app.world().entities().contains(entity));
        assert!(app.world().get::<BuildProject>(entity).is_none());
        assert_eq!(city_count(&mut app), 1);
    }

    #[test]
    fn test_resolve_builds_allows_city_at_minimum_distance() {
        use bevy::prelude::*;
        use shared::cities::City;
        use shared::hex::HexPosition;
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, resolve_builds);
        let player = app
            .world_mut()
            .spawn(Player {
                color_index: 0,
                gold: 0,
                slot_index: 0,
            })
            .id();
        app.world_mut().spawn((City, HexPosition::new(0, 0)));
        let entity = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(4, 0),
                BuildProject {
                    name: "city".into(),
                },
                Owner(player),
                ColorIndex(0),
            ))
            .id();

        app.update();

        assert!(!app.world().entities().contains(entity));
        assert_eq!(city_count(&mut app), 2);
    }

    #[test]
    fn test_resolve_builds_rejects_same_turn_city_too_close_to_new_city() {
        use bevy::prelude::*;
        use shared::hex::HexPosition;
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, resolve_builds);
        let player = app
            .world_mut()
            .spawn(Player {
                color_index: 0,
                gold: 0,
                slot_index: 0,
            })
            .id();
        let first = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(0, 0),
                BuildProject {
                    name: "city".into(),
                },
                Owner(player),
                ColorIndex(0),
            ))
            .id();
        let second = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(3, 0),
                BuildProject {
                    name: "city".into(),
                },
                Owner(player),
                ColorIndex(0),
            ))
            .id();

        app.update();

        assert!(!app.world().entities().contains(first));
        assert!(app.world().entities().contains(second));
        assert!(app.world().get::<BuildProject>(second).is_none());
        assert_eq!(city_count(&mut app), 1);
    }

    #[test]
    fn test_resolve_builds_removes_marker() {
        use bevy::prelude::*;
        use shared::hex::HexPosition;
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, resolve_builds);
        let player = app
            .world_mut()
            .spawn(Player {
                color_index: 0,
                gold: 0,
                slot_index: 0,
            })
            .id();
        let entity = app
            .world_mut()
            .spawn((
                Unit {
                    type_id: UnitTypeId(0),
                },
                HexPosition::new(0, 0),
                BuildProject {
                    name: "other".into(),
                },
                Owner(player),
                ColorIndex(0),
            ))
            .id();
        app.update();
        assert!(app.world().get::<BuildProject>(entity).is_none());
    }

    fn city_count(app: &mut App) -> usize {
        let mut cities = app.world_mut().query_filtered::<Entity, With<City>>();
        cities.iter(app.world()).count()
    }

    /// Regression test: a unit's own queued MoveTo must not block its own
    /// re-submission to the same target.
    ///
    /// Detecting the bug is awkward because `queue_marker` only runs on
    /// acceptance, so the prior MoveTo persists either way and inspecting
    /// MoveTo alone tells us nothing. Trick: insert a `Fortifying` sentinel
    /// on the mover before the re-submission; `queue_marker` strips every
    /// prior marker when it runs, so the sentinel's absence after re-submit
    /// proves the action was accepted.
    #[test]
    fn test_handle_unit_action_allows_same_unit_to_change_move_target() {
        use crate::players::PlayerMap;
        use bevy::app::ScheduleRunnerPlugin;
        use bevy::state::app::StatesPlugin;
        use bevy_replicon::prelude::*;
        use shared::components::*;
        use shared::events::*;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));

        let warrior_type = UnitTypeId(0);
        let warrior_def = UnitDefinition {
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 4,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry
            .name_to_id
            .insert("warrior".to_string(), warrior_type);
        registry.definitions.insert(warrior_type, warrior_def);

        app.insert_resource(registry);
        app.init_resource::<PlayerState>();
        app.init_resource::<PlayerMap>();
        app.add_observer(handle_unit_action);
        app.update();

        let (client, _player, mover, neighbor) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Accepting,
                turn_number: 1,
            });
            let player = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                    slot_index: 0,
                })
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, player);
            let mover = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(0, 0),
                    Owner(player),
                ))
                .id();
            // Neighbor already has MoveTo(1,0) queued — mover must not target (1,0).
            let neighbor = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(2, 0),
                    Owner(player),
                    MoveTo {
                        pos: HexPosition::new(1, 0),
                    },
                ))
                .id();
            (client, player, mover, neighbor)
        };

        // Part A: confirm the friendly-stacking rule still fires for a DIFFERENT unit.
        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit: mover,
                action: UnitAction::Move {
                    target: HexPosition::new(1, 0),
                },
            },
        });
        app.world_mut().flush();
        assert!(
            app.world().get::<MoveTo>(mover).is_none(),
            "Move to a tile already queued by a friendly should be rejected"
        );

        // Part B: mover gets Move to (-1, 0) accepted (free tile, no conflict).
        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit: mover,
                action: UnitAction::Move {
                    target: HexPosition::new(-1, 0),
                },
            },
        });
        app.world_mut().flush();
        assert!(
            app.world().get::<MoveTo>(mover).is_some(),
            "Move to free tile should be accepted"
        );

        // Manually add a Fortifying marker so we can detect whether queue_marker ran.
        // queue_marker always removes Fortifying before inserting the new marker, so if
        // the re-submission is accepted, Fortifying will be gone. If the bug causes
        // rejection, queue_marker never runs and Fortifying persists.
        app.world_mut().entity_mut(mover).insert(Fortifying);
        app.world_mut().flush();

        // Part C: mover resubmits Move to (-1, 0) — same target as the queued marker.
        // Bug: queued_moves sees the unit's own MoveTo{(-1,0)}, pos matches, rejects it.
        // Fix: the check skips the issuing entity, so this is accepted and queue_marker runs.
        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit: mover,
                action: UnitAction::Move {
                    target: HexPosition::new(-1, 0),
                },
            },
        });
        app.world_mut().flush();

        // If the re-submission was accepted, queue_marker ran and removed Fortifying.
        assert!(
            app.world().get::<Fortifying>(mover).is_none(),
            "re-submitting Move to same target on same unit must be accepted — \
             queue_marker should have run and removed the Fortifying marker; \
             if Fortifying persists, the unit's own queued MoveTo is blocking itself"
        );
        // And the MoveTo should still be present pointing to the same target.
        let mt = app
            .world()
            .get::<MoveTo>(mover)
            .expect("MoveTo must still be present after re-submission");
        assert_eq!(mt.pos, HexPosition::new(-1, 0));

        let _ = neighbor;
    }

    #[test]
    fn test_handle_unit_action_rejects_move_to_friendly_occupied_tile() {
        use crate::players::PlayerMap;
        use bevy::app::ScheduleRunnerPlugin;
        use bevy::state::app::StatesPlugin;
        use bevy_replicon::prelude::*;
        use shared::components::*;
        use shared::events::*;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));

        // Minimal warrior registry.
        let warrior_type = UnitTypeId(0);
        let warrior_def = UnitDefinition {
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 4,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry
            .name_to_id
            .insert("warrior".to_string(), warrior_type);
        registry.definitions.insert(warrior_type, warrior_def);

        app.insert_resource(registry);
        app.init_resource::<PlayerState>();
        app.init_resource::<PlayerMap>();
        app.add_observer(handle_unit_action);
        app.update();

        let (client, player, mover, blocker) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Accepting,
                turn_number: 1,
            });
            let player = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                    slot_index: 0,
                })
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, player);

            // Two friendly warriors. Mover at (0, 0), blocker at (1, 0).
            let mover = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(0, 0),
                    Owner(player),
                ))
                .id();
            let blocker = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(1, 0),
                    Owner(player),
                ))
                .id();
            (client, player, mover, blocker)
        };

        // Try to move mover onto blocker's tile.
        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit: mover,
                action: UnitAction::Move {
                    target: HexPosition::new(1, 0),
                },
            },
        });
        app.world_mut().flush();

        // Move must be rejected: no MoveTo on the mover.
        assert!(
            app.world().get::<MoveTo>(mover).is_none(),
            "Move to friendly-occupied tile must be rejected"
        );
        let _ = (player, blocker);
    }

    #[test]
    fn test_handle_unit_action_rejects_friendly_city_targets() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));

        let archer_type = UnitTypeId(0);
        let archer_def = UnitDefinition {
            hp: 8,
            move_budget: 2,
            attack_range: 2,
            attack_damage: 3,
            gold_upkeep: 1,
            production_cost: 25,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("archer".into(), archer_type);
        registry.definitions.insert(archer_type, archer_def);

        app.insert_resource(registry);
        app.init_resource::<PlayerState>();
        app.init_resource::<PlayerMap>();
        app.add_observer(handle_unit_action);
        app.update();

        let (client, unit) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Accepting,
                turn_number: 1,
            });
            let player = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                    slot_index: 0,
                })
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, player);
            let unit = world
                .spawn((
                    Unit {
                        type_id: archer_type,
                    },
                    HexPosition::new(0, 0),
                    Owner(player),
                ))
                .id();
            world.spawn((City, HexPosition::new(1, 0), CityOwner { entity: player }));
            (client, unit)
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit,
                action: UnitAction::Move {
                    target: HexPosition::new(1, 0),
                },
            },
        });
        app.world_mut().flush();
        assert!(
            app.world().get::<MoveTo>(unit).is_some(),
            "moving into a friendly city must be allowed"
        );

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit,
                action: UnitAction::Attack {
                    target: HexPosition::new(1, 0),
                },
            },
        });
        app.world_mut().flush();
        assert!(
            app.world().get::<MoveTo>(unit).is_some(),
            "rejected attack must preserve the queued friendly-city move"
        );
        assert!(
            app.world().get::<AttackTarget>(unit).is_none(),
            "attacking a friendly city must be rejected"
        );
    }

    #[test]
    fn test_handle_unit_action_rejects_ranged_move_to_enemy_city() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));

        let archer_type = UnitTypeId(0);
        let archer_def = UnitDefinition {
            hp: 8,
            move_budget: 2,
            attack_range: 2,
            attack_damage: 3,
            gold_upkeep: 1,
            production_cost: 25,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("archer".into(), archer_type);
        registry.definitions.insert(archer_type, archer_def);

        app.insert_resource(registry);
        app.init_resource::<PlayerState>();
        app.init_resource::<PlayerMap>();
        app.add_observer(handle_unit_action);
        app.update();

        let (client, unit) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Accepting,
                turn_number: 1,
            });
            let player = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                    slot_index: 0,
                })
                .id();
            let enemy = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                    slot_index: 1,
                })
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, player);
            let unit = world
                .spawn((
                    Unit {
                        type_id: archer_type,
                    },
                    HexPosition::new(0, 0),
                    Owner(player),
                ))
                .id();
            world.spawn((City, HexPosition::new(1, 0), CityOwner { entity: enemy }));
            (client, unit)
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit,
                action: UnitAction::Move {
                    target: HexPosition::new(1, 0),
                },
            },
        });
        app.world_mut().flush();

        assert!(
            app.world().get::<MoveTo>(unit).is_none(),
            "ranged units must not capture cities by moving into them"
        );
    }

    #[test]
    fn test_handle_unit_action_rejects_move_to_tile_with_queued_friendly_move() {
        use crate::players::PlayerMap;
        use bevy::app::ScheduleRunnerPlugin;
        use bevy::state::app::StatesPlugin;
        use bevy_replicon::prelude::*;
        use shared::components::*;
        use shared::events::*;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));

        let warrior_type = UnitTypeId(0);
        let warrior_def = UnitDefinition {
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 4,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry
            .name_to_id
            .insert("warrior".to_string(), warrior_type);
        registry.definitions.insert(warrior_type, warrior_def);

        app.insert_resource(registry);
        app.init_resource::<PlayerState>();
        app.init_resource::<PlayerMap>();
        app.add_observer(handle_unit_action);
        app.update();

        let (client, _player, first, second) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Accepting,
                turn_number: 1,
            });
            let player = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                    slot_index: 0,
                })
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, player);

            // Two friendly warriors at distinct tiles, both able to reach (2, 0).
            let first = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(1, 0),
                    Owner(player),
                ))
                .id();
            let second = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(3, 0),
                    Owner(player),
                ))
                .id();
            (client, player, first, second)
        };

        // First friendly queues Move to (2, 0). Should succeed.
        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit: first,
                action: UnitAction::Move {
                    target: HexPosition::new(2, 0),
                },
            },
        });
        app.world_mut().flush();
        assert!(
            app.world().get::<MoveTo>(first).is_some(),
            "First friendly's Move should be accepted"
        );

        // Second friendly queues Move to the same tile. Should be rejected.
        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: UnitActionEvent {
                unit: second,
                action: UnitAction::Move {
                    target: HexPosition::new(2, 0),
                },
            },
        });
        app.world_mut().flush();
        assert!(
            app.world().get::<MoveTo>(second).is_none(),
            "Second friendly's Move to a tile already targeted must be rejected"
        );
    }

    #[test]
    fn update_turn_phase_resets_to_lobby_when_all_players_leave() {
        let mut app = App::new();
        app.add_systems(Update, update_turn_phase);
        app.world_mut().spawn(TurnState {
            phase: TurnPhase::WaitingForPlayers,
            turn_number: 3,
        });
        // No Player entities — simulates all-disconnected state.
        app.update();

        let mut q = app.world_mut().query::<&TurnState>();
        let state = q.single(app.world()).unwrap();
        assert_eq!(state.phase, TurnPhase::Lobby, "phase must reset to Lobby");
        assert_eq!(state.turn_number, 0, "turn_number must reset to 0");
    }

    fn lobby_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));
        app.init_resource::<PlayerMap>();
        app.add_observer(handle_start_game);
        app.update();
        app
    }

    #[test]
    fn handle_start_game_host_with_two_players_transitions_to_accepting() {
        let mut app = lobby_app();

        let (client, _player) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Lobby,
                turn_number: 0,
            });
            let player1 = world
                .spawn((
                    Player {
                        color_index: 0,
                        gold: 0,
                        slot_index: 0,
                    },
                    Host,
                ))
                .id();
            let _player2 = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                    slot_index: 1,
                })
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, player1);
            (client, player1)
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: StartGame,
        });
        app.world_mut().flush();

        let mut q = app.world_mut().query::<&TurnState>();
        let state = q.single(app.world()).unwrap();
        assert_eq!(
            state.phase,
            TurnPhase::Accepting,
            "host StartGame with 2 players must transition to Accepting"
        );
    }

    #[test]
    fn handle_start_game_rejects_non_host_sender() {
        let mut app = lobby_app();

        let (non_host_client, _host_player, _non_host_player) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Lobby,
                turn_number: 0,
            });
            let host_player = world
                .spawn((
                    Player {
                        color_index: 0,
                        gold: 0,
                        slot_index: 0,
                    },
                    Host,
                ))
                .id();
            let non_host_player = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                    slot_index: 1,
                })
                .id();
            let non_host_client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(non_host_client, non_host_player);
            (non_host_client, host_player, non_host_player)
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(non_host_client),
            message: StartGame,
        });
        app.world_mut().flush();

        let mut q = app.world_mut().query::<&TurnState>();
        let state = q.single(app.world()).unwrap();
        assert_eq!(
            state.phase,
            TurnPhase::Lobby,
            "non-host StartGame must be rejected; phase must remain Lobby"
        );
    }

    #[test]
    fn handle_start_game_rejects_when_only_one_player() {
        let mut app = lobby_app();

        let (client, _player) = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Lobby,
                turn_number: 0,
            });
            let player = world
                .spawn((
                    Player {
                        color_index: 0,
                        gold: 0,
                        slot_index: 0,
                    },
                    Host,
                ))
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, player);
            (client, player)
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: StartGame,
        });
        app.world_mut().flush();

        let mut q = app.world_mut().query::<&TurnState>();
        let state = q.single(app.world()).unwrap();
        assert_eq!(
            state.phase,
            TurnPhase::Lobby,
            "StartGame with only 1 player must be rejected"
        );
    }
}
