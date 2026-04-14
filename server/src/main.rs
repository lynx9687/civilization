use std::{
    net::{SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::{app::ScheduleRunnerPlugin, prelude::*, state::app::StatesPlugin};
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig},
    renet::ConnectionConfig,
    RenetChannelsExt, RenetServer, RepliconRenetPlugins,
};
use shared::{
    components::*,
    hex::generate_grid,
    plugin::SharedPlugin,
};

const PROTOCOL_ID: u64 = 0;
const GRID_RADIUS: i32 = 5;

#[derive(Resource)]
struct BindAddr(SocketAddr);

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
        .add_systems(Startup, (start_server, spawn_grid))
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

    println!("Spawned grid with radius {GRID_RADIUS} ({} tiles)", 3 * GRID_RADIUS * GRID_RADIUS + 3 * GRID_RADIUS + 1);
}
