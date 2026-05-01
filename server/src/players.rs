use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use rand::Rng;
use shared::events::*;
use shared::unit_definition::UnitRegistry;
use shared::{components::*, hex::HexPosition, units::*};

use crate::turn::{PendingMoves, PlayerState, PlayerTurnState};

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

/// Assigns unique ids to players
#[derive(Resource, Default)]
pub struct PlayerCounter(u32);

impl PlayerCounter {
    pub fn next_id(&mut self) -> u32 {
        let res = self.0;
        self.0 += 1;
        res
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_new_clients(
    new_clients: Query<Entity, Added<AuthorizedClient>>,
    mut commands: Commands,
    mut player_map: ResMut<PlayerMap>,
    mut color_counter: ResMut<ColorCounter>,
    mut player_counter: ResMut<PlayerCounter>,
    mut unit_counter: ResMut<UnitCounter>,
    registry: Res<UnitRegistry>,
    mut player_state: ResMut<PlayerState>,
) {
    for client_entity in &new_clients {
        let color_index = color_counter.next_index();
        let player_id = player_counter.next_id();
        let player_entity = commands
            .spawn((
                Player {
                    player_id,
                    color_index,
                },
                HexPosition::new(0, 0),
            ))
            .id();

        player_map
            .client_to_player
            .insert(client_entity, player_entity);

        player_state
            .turn
            .insert(client_entity, crate::turn::PlayerTurnState::InProgress);

        let client_id = ClientId::Client(client_entity);
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(client_id),
            message: YourPlayer {
                player_id,
                color_index,
            },
        });

        println!("Player joined (color {color_index}), entity: {player_entity}");

        let starting_units = ["warrior", "settler"];
        for unit_type in starting_units {
            let definition = registry
                .get(unit_type)
                .unwrap_or_else(|| panic!("missing unit definition for {unit_type}"));
            let unit_id = unit_counter.next_id();
            let x = rand::thread_rng().gen_range(-2..=2);
            let y = rand::thread_rng().gen_range(-2..=2);
            let unit_entity = commands
                .spawn((
                    Unit {
                        id: unit_id,
                        type_name: unit_type.to_string(),
                    },
                    HexPosition::new(x, y),
                    Owner { player_id },
                    ColorIndex(color_index),
                    Health::full(definition.hp),
                ))
                .id();
            println!(
                "Spawned {unit_type}: {unit_entity} (HP {}) for player: {player_entity}",
                definition.hp
            );
        }
    }
}

pub fn handle_disconnects(
    mut disconnected: RemovedComponents<ConnectedClient>,
    mut player_map: ResMut<PlayerMap>,
    mut commands: Commands,
    mut pending_moves: ResMut<PendingMoves>,
    mut player_state: ResMut<PlayerState>,
) {
    for client_entity in disconnected.read() {
        let prev_state = player_state.turn.remove(&client_entity);
        if prev_state.is_some_and(|state| state == PlayerTurnState::Finished) {
            player_state.finished_cnt -= 1;
        }
        if let Some(player_entity) = player_map.client_to_player.remove(&client_entity) {
            pending_moves.moves.remove(&player_entity);
            commands.entity(player_entity).despawn();
            println!("Player disconnected, despawned {player_entity}");
        }
    }
}
