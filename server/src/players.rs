use std::collections::{HashMap, HashSet};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use rand::seq::SliceRandom;
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
#[derive(Resource, Default)]
pub struct PlayerMap {
    pub client_to_player: HashMap<Entity, Entity>,
}

/// Tracks next color index to assign.
#[derive(Resource, Default)]
pub struct ColorCounter(u8);

impl ColorCounter {
    pub fn next_index(&mut self) -> u8 {
        let idx = self.0;
        self.0 = (self.0 + 1) % 8;
        idx
    }
}

#[derive(SystemParam)]
pub struct NewPlayerSetup<'w> {
    player_map: ResMut<'w, PlayerMap>,
    player_state: ResMut<'w, PlayerState>,
}

#[allow(clippy::too_many_arguments)]
pub fn handle_new_clients(
    new_clients: Query<Entity, Added<AuthorizedClient>>,
    existing_units: Query<&HexPosition, With<Unit>>,
    mut commands: Commands,
    mut color_counter: ResMut<ColorCounter>,
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
        let color_index = color_counter.next_index();
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

        setup
            .player_state
            .turn
            .insert(client_entity, crate::turn::PlayerTurnState::InProgress);

        let client_id = ClientId::Client(client_entity);
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(client_id),
            message: YourPlayer {
                player_entity,
                color_index,
            },
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

pub fn handle_disconnects(
    mut disconnected: RemovedComponents<ConnectedClient>,
    mut player_map: ResMut<PlayerMap>,
    mut commands: Commands,
    mut player_state: ResMut<PlayerState>,
    host_check: Query<(), With<Host>>,
) {
    for client_entity in disconnected.read() {
        let prev_state = player_state.turn.remove(&client_entity);
        if prev_state.is_some_and(|state| state == PlayerTurnState::Finished) {
            player_state.finished_cnt -= 1;
        }
        if let Some(player_entity) = player_map.client_to_player.remove(&client_entity) {
            let was_host = host_check.contains(player_entity);
            commands.entity(player_entity).despawn();

            if was_host {
                // client_to_player already has this client removed, so values() gives remaining players
                if let Some(&next_player) = player_map.client_to_player.values().next() {
                    commands.entity(next_player).insert(Host);
                    println!("Host transferred to {next_player}");
                }
            }

            println!("Player disconnected, despawned {player_entity}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use std::collections::HashSet;

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
