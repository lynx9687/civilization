use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::events::*;
use shared::unit_definition::{
    UnitRegistry, available_verbs, is_within_attack_range, is_within_move_range,
};
use shared::units::*;
use shared::{components::*, hex::HexPosition};

use crate::GRID_RADIUS;
use crate::players::PlayerMap;

/// Collects submitted moves during the Accepting phase, then drains them all at once to resolve the turn.
/// Maps player entity → target hex.
#[derive(Resource, Default)]
pub struct PendingMoves {
    pub moves: HashMap<Entity, HexPosition>,
}

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

pub fn update_turn_phase(
    players: Query<(), With<Player>>,
    mut turn_state: Query<&mut TurnState>,
    mut pending_moves: ResMut<PendingMoves>,
) {
    let count = players.iter().count();
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };

    if count < 2 {
        if state.phase != TurnPhase::WaitingForPlayers {
            state.phase = TurnPhase::WaitingForPlayers;
            pending_moves.moves.clear();
            println!("Not enough players ({count}), waiting...");
        }
    } else if state.phase == TurnPhase::WaitingForPlayers {
        state.phase = TurnPhase::Accepting;
        println!(
            "Enough players ({count}), accepting moves for turn {}",
            state.turn_number
        );
    }
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
            queue_marker(&mut commands, entity, MoveTo { pos: *target });
        }
        UnitAction::Attack { target } => {
            if !is_within_attack_range(pos, target, def.attack_range) {
                println!("Rejected attack: out of range");
                return;
            }
            let enemy_here = enemy_units
                .iter()
                .any(|(p, o)| p == target && o.0 != *player_entity);
            if !enemy_here {
                println!("Rejected attack: no enemy at target");
                return;
            }
            queue_marker(&mut commands, entity, AttackTarget { pos: *target });
        }
        UnitAction::Fortify => {
            queue_marker(&mut commands, entity, Fortifying);
        }
        UnitAction::Skip => {
            queue_marker(&mut commands, entity, Skipping);
        }
        UnitAction::Build { project } => {
            if !def.build_targets.contains(project) {
                println!("Rejected build: project {project:?} not buildable");
                return;
            }
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

pub fn resolve_turn(
    players: Query<Entity, With<Player>>,
    mut units: Query<(Entity, &MoveTo, &mut HexPosition), With<MoveTo>>,
    mut commands: Commands,
    mut turn_state: Query<&mut TurnState>,
    mut player_state: ResMut<PlayerState>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let player_count = players.iter().count() as i32;
    if player_count < 2 || player_state.finished_cnt < player_count {
        return;
    }

    // Apply all moves simultaneously
    for (entity, move_to, mut pos) in &mut units {
        *pos = move_to.pos;
        commands.entity(entity).remove::<MoveTo>();
    }

    // Advance turn
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };
    state.turn_number += 1;
    //reset finished players
    player_state.finished_cnt = 0;
    for val in player_state.turn.values_mut() {
        *val = PlayerTurnState::InProgress;
    }
    println!("Turn resolved! Now on turn {}", state.turn_number);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bevy::app::ScheduleRunnerPlugin;
    use bevy::state::app::StatesPlugin;
    use bevy_replicon::prelude::*;
    use shared::components::*;
    use shared::events::*;
    use shared::unit_definition::*;
    use shared::units::*;

    use super::*;
    use crate::players::PlayerMap;

    #[test]
    fn test_pending_moves_tracking() {
        let mut pending = PendingMoves::default();
        let entity = Entity::from_bits(1);
        let target = HexPosition::new(1, 0);

        assert!(!pending.moves.contains_key(&entity));
        pending.moves.insert(entity, target);
        assert!(pending.moves.contains_key(&entity));
        assert_eq!(pending.moves.len(), 1);

        pending.moves.drain();
        assert_eq!(pending.moves.len(), 0);
    }

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
        registry.name_to_id.insert("warrior".to_string(), warrior_type);
        registry.definitions.insert(warrior_type, warrior_def);

        app.insert_resource(registry);
        app.init_resource::<PendingMoves>();
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
            let player_entity = world.spawn(Player { player_id: 0, color_index: 0 }).id();

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
}
