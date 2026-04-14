use std::{
    net::{SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::{app::ScheduleRunnerPlugin, prelude::*, state::app::StatesPlugin};
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    RenetChannelsExt, RenetServer, RepliconRenetPlugins,
    netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig},
    renet::ConnectionConfig,
};
use shared::{
    components::*,
    hex::{HexPosition, generate_grid},
    plugin::SharedPlugin,
};

const PROTOCOL_ID: u64 = 0;
const GRID_RADIUS: i32 = 5;

#[derive(Resource)]
struct BindAddr(SocketAddr);

/// Maps ConnectedClient entity → Player entity.
#[derive(Resource, Default)]
struct PlayerMap {
    client_to_player: std::collections::HashMap<Entity, Entity>,
}

/// Tracks next color index to assign.
#[derive(Resource, Default)]
struct ColorCounter(u8);

impl ColorCounter {
    fn next(&mut self) -> u8 {
        let idx = self.0;
        self.0 = (self.0 + 1) % 8;
        idx
    }
}

#[derive(Resource, Default)]
struct PendingMoves {
    moves: std::collections::HashMap<Entity, HexPosition>,
}

fn main() {
    let addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0.0.0.0:5000".to_string());
    let addr: SocketAddr = addr_str.parse().expect("Invalid bind address");

    println!("Starting server on {addr}");

    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            )),
            StatesPlugin,
            RepliconPlugins,
            RepliconRenetPlugins,
            SharedPlugin,
        ))
        .insert_resource(BindAddr(addr))
        .init_resource::<PlayerMap>()
        .init_resource::<ColorCounter>()
        .init_resource::<PendingMoves>()
        .add_systems(Startup, (start_server, spawn_grid))
        .add_observer(handle_move)
        .add_systems(
            Update,
            (
                handle_new_clients,
                handle_disconnects,
                update_turn_phase,
                resolve_turn,
            )
                .chain(),
        )
        .run();
}

fn start_server(mut commands: Commands, channels: Res<RepliconChannels>, addr: Res<BindAddr>) {
    let server_channels_config = channels.server_configs();
    let client_channels_config = channels.client_configs();

    let server = RenetServer::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let socket = UdpSocket::bind(addr.0).unwrap();
    let server_config = ServerConfig {
        current_time,
        max_clients: 8,
        protocol_id: PROTOCOL_ID,
        authentication: ServerAuthentication::Unsecure,
        public_addresses: Default::default(),
    };
    let transport = NetcodeServerTransport::new(server_config, socket).unwrap();

    commands.insert_resource(server);
    commands.insert_resource(transport);

    println!("Server listening on {}", addr.0);
}

fn spawn_grid(mut commands: Commands) {
    // Spawn hex tile entities
    for pos in generate_grid(GRID_RADIUS) {
        commands.spawn((Replicated, HexTile, pos));
    }

    // Spawn turn state entity
    commands.spawn((
        Replicated,
        TurnState {
            phase: TurnPhase::WaitingForPlayers,
            turn_number: 0,
        },
    ));

    println!(
        "Spawned grid with radius {GRID_RADIUS} ({} tiles)",
        3 * GRID_RADIUS * GRID_RADIUS + 3 * GRID_RADIUS + 1
    );
}

fn handle_new_clients(
    new_clients: Query<Entity, Added<ConnectedClient>>,
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
    }
}

fn handle_disconnects(
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

fn update_turn_phase(
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

fn handle_move(
    trigger: On<FromClient<MoveAction>>,
    player_map: Res<PlayerMap>,
    players: Query<&HexPosition, With<Player>>,
    mut pending_moves: ResMut<PendingMoves>,
    turn_state: Query<&TurnState>,
) {
    let client_entity = match trigger.client_id {
        ClientId::Client(entity) => entity,
        ClientId::Server => return,
    };
    let target = trigger.message.target;

    let Some(&player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };

    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    if pending_moves.moves.contains_key(&player_entity) {
        return; // already submitted this turn
    }

    let Ok(current_pos) = players.get(player_entity) else {
        return;
    };

    if !current_pos.is_neighbor(&target) {
        println!(
            "Rejected move: {:?} is not a neighbor of {:?}",
            target, current_pos
        );
        return;
    }
    if !target.in_bounds(GRID_RADIUS) {
        println!("Rejected move: {:?} is out of bounds", target);
        return;
    }

    pending_moves.moves.insert(player_entity, target);
    println!(
        "Move accepted: player {player_entity} -> {:?} ({}/?)",
        target,
        pending_moves.moves.len()
    );
}

fn resolve_turn(
    mut pending_moves: ResMut<PendingMoves>,
    players: Query<Entity, With<Player>>,
    mut positions: Query<&mut HexPosition, With<Player>>,
    mut turn_state: Query<&mut TurnState>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let player_count = players.iter().count();
    if player_count < 2 || pending_moves.moves.len() < player_count {
        return;
    }

    // Apply all moves simultaneously
    for (entity, target) in pending_moves.moves.drain() {
        if let Ok(mut pos) = positions.get_mut(entity) {
            *pos = target;
        }
    }

    // Advance turn
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };
    state.turn_number += 1;
    println!("Turn resolved! Now on turn {}", state.turn_number);
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::hex::HexPosition;

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
}
