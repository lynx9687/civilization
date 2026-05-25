use std::collections::{HashMap, HashSet};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use rand::seq::SliceRandom;
use shared::cities::{City, CityOwner};
use shared::events::*;
use shared::unit_definition::UnitRegistry;
use shared::{components::*, hex::HexPosition, units::*};

use crate::turn::{PlayerState, PlayerTurnState};

/// Half-extent (in axial coords) of the player-starting region.
/// Region is the axial parallelogram q,r ∈ [-N, N] — a (2N+1)² parallelogram, NOT a hex disc.
/// At N=2 the region has 25 tiles. With max_clients=8 and up to 3 starting units per player
/// (24 total), this is tight but fits — bump if either rises.
const STARTING_AREA_HALF_EXTENT: i32 = 2;

fn pick_starting_positions(
    occupied: &HashSet<HexPosition>,
    count: usize,
    rng: &mut impl rand::Rng,
) -> Vec<HexPosition> {
    let mut candidates = Vec::new();
    for q in -STARTING_AREA_HALF_EXTENT..=STARTING_AREA_HALF_EXTENT {
        for r in -STARTING_AREA_HALF_EXTENT..=STARTING_AREA_HALF_EXTENT {
            let pos = HexPosition::new(q, r);
            if !occupied.contains(&pos) {
                candidates.push(pos);
            }
        }
    }
    if candidates.len() < count {
        panic!(
            "not enough free tiles for {count} starting positions: {} candidate(s) available in spawn region (half-extent {STARTING_AREA_HALF_EXTENT})",
            candidates.len()
        );
    }
    candidates.shuffle(rng);
    candidates.into_iter().take(count).collect()
}

/// Maps ConnectedClient entity → Player entity.
/// `join_order` holds client entities in strict connection order so that
/// host reassignment always picks the oldest remaining connected player.
#[derive(Resource, Default)]
pub struct PlayerMap {
    pub client_to_player: HashMap<Entity, Entity>,
    pub join_order: Vec<Entity>,
}

#[derive(SystemParam)]
pub struct NewPlayerSetup<'w> {
    player_map: ResMut<'w, PlayerMap>,
    player_state: ResMut<'w, PlayerState>,
}

pub fn handle_new_clients(
    new_clients: Query<Entity, Added<AuthorizedClient>>,
    existing_units: Query<&HexPosition, With<Unit>>,
    mut commands: Commands,
    registry: Res<UnitRegistry>,
    mut setup: NewPlayerSetup,
    hosts: Query<(), With<Host>>,
) {
    // Seed the occupied set from existing units; updated in-line as we
    // assign positions so two clients added in the same frame don't collide
    // (Commands::spawn doesn't materialize until the next system run).
    let mut occupied: HashSet<HexPosition> = existing_units.iter().copied().collect();
    let mut rng = rand::thread_rng();
    // Prevents two clients joining the same frame from both becoming host.
    let mut host_assigned_this_frame = false;

    for client_entity in &new_clients {
        let color_index = setup.player_map.join_order.len() as u8;
        let player_entity = commands
            .spawn((
                Player {
                    color_index,
                    gold: 0,
                },
                HexPosition::new(0, 0),
            ))
            .id();

        if !host_assigned_this_frame && hosts.is_empty() {
            commands.entity(player_entity).insert(Host);
            host_assigned_this_frame = true;
            println!("Player {player_entity} is HOST");
        }

        setup
            .player_map
            .client_to_player
            .insert(client_entity, player_entity);
        setup.player_map.join_order.push(client_entity);

        setup
            .player_state
            .turn
            .insert(client_entity, crate::turn::PlayerTurnState::InProgress);

        let client_id = ClientId::Client(client_entity);
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(client_id),
            message: YourPlayer { player_entity },
        });

        println!("Player joined (color {color_index}), entity: {player_entity}");

        let starting_units = ["warrior", "archer", "settler", "knight", "cavalry"];
        let positions = pick_starting_positions(&occupied, starting_units.len(), &mut rng);

        for (unit_type, pos) in starting_units.iter().zip(positions.iter()) {
            let type_id = registry
                .id_of(unit_type)
                .unwrap_or_else(|| panic!("missing unit definition for {unit_type}"));
            let definition = registry
                .get(&type_id)
                .unwrap_or_else(|| panic!("registry has id but no definition for {unit_type}"));
            let unit_entity = commands
                .spawn((
                    Unit { type_id },
                    *pos,
                    Owner(player_entity),
                    ColorIndex(color_index),
                    Health::full(definition.hp),
                ))
                .id();
            occupied.insert(*pos);
            println!(
                "Spawned {unit_type}: {unit_entity} at ({}, {}) (HP {}) for player: {player_entity}",
                pos.q, pos.r, definition.hp
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn handle_disconnects(
    mut disconnected: RemovedComponents<ConnectedClient>,
    mut player_map: ResMut<PlayerMap>,
    mut commands: Commands,
    mut player_state: ResMut<PlayerState>,
    host_check: Query<(), With<Host>>,
    mut players_query: Query<&mut Player>,
    mut unit_colors: Query<(&Owner, &mut ColorIndex), (With<Unit>, Without<City>)>,
    mut city_colors: Query<(&CityOwner, &mut ColorIndex), (With<City>, Without<Unit>)>,
) {
    for client_entity in disconnected.read() {
        let prev_state = player_state.turn.remove(&client_entity);
        if prev_state.is_some_and(|state| state == PlayerTurnState::Finished) {
            player_state.finished_cnt -= 1;
        }
        if let Some(player_entity) = player_map.client_to_player.remove(&client_entity) {
            // Keep join_order in sync; do this before the host-reassignment check
            // so that join_order.first() already reflects the remaining players.
            player_map.join_order.retain(|&e| e != client_entity);

            let was_host = host_check.contains(player_entity);
            commands.entity(player_entity).despawn();

            if was_host {
                // Oldest remaining connected player (join_order[0]) becomes host.
                if let Some(&oldest_client) = player_map.join_order.first()
                    && let Some(&next_player) = player_map.client_to_player.get(&oldest_client)
                {
                    commands.entity(next_player).insert(Host);
                    println!("Host transferred to {next_player}");
                }
            }

            println!("Player disconnected, despawned {player_entity}");
        }
    }

    // Reassign color indices so the lobby list stays contiguous after any disconnect.
    // Collect (player_entity → new_color) first so we can update units/cities in the same pass.
    let mut color_map: HashMap<Entity, u8> = HashMap::new();
    for (idx, &client_entity) in player_map.join_order.iter().enumerate() {
        if let Some(&player_entity) = player_map.client_to_player.get(&client_entity)
            && let Ok(mut player) = players_query.get_mut(player_entity)
        {
            player.color_index = idx as u8;
            color_map.insert(player_entity, idx as u8);
        }
    }
    // Keep ColorIndex in sync so in-game colors match lobby color indices.
    for (owner, mut color) in &mut unit_colors {
        if let Some(&new_color) = color_map.get(&owner.0) {
            color.0 = new_color;
        }
    }
    for (city_owner, mut color) in &mut city_colors {
        if let Some(&new_color) = color_map.get(&city_owner.entity) {
            color.0 = new_color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use std::collections::HashSet;

    /// join_order must reflect insertion order and drop removed entries.
    #[test]
    fn join_order_tracks_connection_order_and_shrinks_on_remove() {
        let mut world = bevy::prelude::World::new();
        let c1 = world.spawn_empty().id();
        let c2 = world.spawn_empty().id();
        let c3 = world.spawn_empty().id();
        let p1 = world.spawn_empty().id();
        let p2 = world.spawn_empty().id();
        let p3 = world.spawn_empty().id();

        let mut map = PlayerMap::default();
        map.client_to_player.insert(c1, p1);
        map.join_order.push(c1);
        map.client_to_player.insert(c2, p2);
        map.join_order.push(c2);
        map.client_to_player.insert(c3, p3);
        map.join_order.push(c3);

        // Remove the first (oldest) client — simulates host disconnect.
        map.client_to_player.remove(&c1);
        map.join_order.retain(|&e| e != c1);

        // Oldest remaining must be c2 (second to join), not c3.
        assert_eq!(
            map.join_order.first().copied(),
            Some(c2),
            "oldest remaining client must be c2"
        );
        assert_eq!(
            map.client_to_player.get(map.join_order.first().unwrap()),
            Some(&p2),
            "host must transfer to player entity of c2"
        );
    }

    #[test]
    fn pick_starting_positions_returns_distinct_when_occupied_empty() {
        let occupied: HashSet<HexPosition> = HashSet::new();
        let mut rng = StdRng::seed_from_u64(0);

        let positions = pick_starting_positions(&occupied, 3, &mut rng);

        assert_eq!(positions.len(), 3);
        let unique: HashSet<HexPosition> = positions.iter().copied().collect();
        assert_eq!(unique.len(), 3, "positions must be distinct");
        for p in &positions {
            assert!(p.q.abs() <= STARTING_AREA_HALF_EXTENT);
            assert!(p.r.abs() <= STARTING_AREA_HALF_EXTENT);
        }
    }

    #[test]
    fn pick_starting_positions_excludes_occupied() {
        let occupied: HashSet<HexPosition> = [
            HexPosition::new(0, 0),
            HexPosition::new(1, 0),
            HexPosition::new(-1, 0),
            HexPosition::new(0, 1),
            HexPosition::new(0, -1),
        ]
        .into_iter()
        .collect();
        let mut rng = StdRng::seed_from_u64(0);

        let positions = pick_starting_positions(&occupied, 3, &mut rng);

        assert_eq!(positions.len(), 3);
        let unique: HashSet<HexPosition> = positions.iter().copied().collect();
        assert_eq!(unique.len(), 3, "positions must be distinct");
        for p in &positions {
            assert!(!occupied.contains(p), "{p:?} should not be in occupied");
            assert!(p.q.abs() <= STARTING_AREA_HALF_EXTENT);
            assert!(p.r.abs() <= STARTING_AREA_HALF_EXTENT);
        }
    }

    #[test]
    #[should_panic(expected = "starting positions")]
    fn pick_starting_positions_panics_when_saturated() {
        // Fill the 25-tile region except 2 tiles, then ask for 3.
        let mut occupied: HashSet<HexPosition> = HashSet::new();
        for q in -STARTING_AREA_HALF_EXTENT..=STARTING_AREA_HALF_EXTENT {
            for r in -STARTING_AREA_HALF_EXTENT..=STARTING_AREA_HALF_EXTENT {
                occupied.insert(HexPosition::new(q, r));
            }
        }
        occupied.remove(&HexPosition::new(0, 0));
        occupied.remove(&HexPosition::new(1, 0));
        let mut rng = StdRng::seed_from_u64(0);

        let _ = pick_starting_positions(&occupied, 3, &mut rng);
    }
}
