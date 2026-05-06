mod cities;
mod cities_systems;
mod players;
mod turn;

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
use shared::{components::*, hex::generate_grid, plugin::SharedPlugin};

use cities::*;
use cities_systems::*;
use players::*;
use turn::*;

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
        .init_resource::<PlayerMap>()
        .init_resource::<ColorCounter>()
        .init_resource::<PlayerState>()
        .add_systems(Startup, (start_server, spawn_grid))
        .add_observer(handle_unit_action)
        .add_observer(handle_finish_turn)
        .add_observer(claim_city_tiles)
        .add_systems(
            Update,
            (
                handle_new_clients,
                handle_disconnects,
                update_turn_phase,
                recalculate_city_yields.run_if(any_city_yields_need_recalculation),
                // resolution window: gated as a group so all resolvers see
                // a consistent "turn end" world; advance_turn closes the window.
                (
                    grow_city_population,
                    grant_city_gold,
                    resolve_moves,
                    resolve_attacks,
                    resolve_fortify,
                    resolve_skip,
                    resolve_builds,
                    advance_turn,
                )
                    .chain()
                    .run_if(turn_is_resolving),
            )
                .chain(),
        )
        .run();
}

fn start_server(
    mut commands: Commands,
    channels: Res<RepliconChannels>,
    addr: Res<BindAddr>,
) -> Result<()> {
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
    let socket = UdpSocket::bind(addr.0)?;
    let server_config = ServerConfig {
        current_time,
        max_clients: 8,
        protocol_id: PROTOCOL_ID,
        authentication: ServerAuthentication::Unsecure,
        public_addresses: Default::default(),
    };
    let transport = NetcodeServerTransport::new(server_config, socket)?;

    commands.insert_resource(server);
    commands.insert_resource(transport);

    println!("Server listening on {}", addr.0);
    Ok(())
}

fn spawn_grid(mut commands: Commands) {
    for pos in generate_grid(GRID_RADIUS) {
        commands.spawn((Replicated, HexTile, pos, DEFAULT_TILE_RESOURCES));
    }

    commands.spawn((TurnState {
        phase: TurnPhase::WaitingForPlayers,
        turn_number: 0,
    },));

    println!(
        "Spawned grid with radius {GRID_RADIUS} ({} tiles)",
        3 * GRID_RADIUS * GRID_RADIUS + 3 * GRID_RADIUS + 1
    );
}
