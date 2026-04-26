use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::{components::*, hex::HexPosition, units::*};
use shared::events::*;

use crate::turn::PendingMoves;

/// Maps ConnectedClient entity → Player entity.
#[derive(Resource, Default)]
pub struct PlayerMap {
    pub client_to_player: HashMap<Entity, Entity>
}

/// Tracks next color index to assign.
#[derive(Resource, Default)]
pub struct ColorCounter(u8);

impl ColorCounter {
    pub fn next(&mut self) -> u8 {
        let idx = self.0;
        self.0 = (self.0 + 1) % 8;
        idx
    }
}

pub fn handle_new_clients(
    new_clients: Query<Entity, Added<AuthorizedClient>>,
    mut commands: Commands,
    mut player_map: ResMut<PlayerMap>,
    mut color_counter: ResMut<ColorCounter>,
) {
    for client_entity in &new_clients {
        let color_index = color_counter.next();
        let player_entity = commands
            .spawn((Replicated, Player { color_index }, HexPosition::new(0, 0)))
            .id();

        player_map
            .client_to_player
            .insert(client_entity, player_entity);

        let client_id = ClientId::Client(client_entity);
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(client_id),
            message: YourPlayer { color_index },
        });

        println!("Player joined (color {color_index}), entity: {player_entity}");

        let unit_entity = commands
            .spawn((Replicated, Unit, HexPosition::new(1, 1), Owner(player_entity), ColorIndex(color_index)))
            .id();

        println!("Spawned unit: {unit_entity}, for player: {player_entity}");
    }
}

pub fn handle_disconnects(
    mut disconnected: RemovedComponents<ConnectedClient>,
    mut player_map: ResMut<PlayerMap>,
    mut commands: Commands,
    mut pending_moves: ResMut<PendingMoves>,
) {
    for client_entity in disconnected.read() {
        if let Some(player_entity) = player_map.client_to_player.remove(&client_entity) {
            pending_moves.moves.remove(&player_entity);
            commands.entity(player_entity).despawn();
            println!("Player disconnected, despawned {player_entity}");
        }
    }
}
