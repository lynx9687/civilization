mod cities;
mod cities_systems;
mod combat;
mod map_config;
mod map_gen;
mod players;
mod turn;

use std::{
    net::{SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::{app::ScheduleRunnerPlugin, prelude::*, state::app::StatesPlugin};
use bevy_replicon::prelude::*;
use bevy_replicon_renet2::{
    RenetChannelsExt, RepliconRenetPlugins,
    netcode::{NativeSocket, NetcodeServerTransport, ServerAuthentication, ServerSetupConfig},
    renet2::{ConnectionConfig, RenetServer},
};
use shared::{components::*, map_settings::MapSettings, plugin::SharedPlugin};

use cities::*;
use cities_systems::*;
use combat::{cleanup_dead_units, resolve_movement, resolve_ranged_attacks};
use map_config::handle_set_map_config;
use map_gen::{
    MapTiles, cleanup_map_on_lobby, generate_map_on_start, should_cleanup_map, should_generate_map,
};
use players::{PlayerMap, handle_disconnects, handle_new_clients, promote_waiting_players};
use turn::*;

const PROTOCOL_ID: u64 = 0;

#[derive(Resource)]
struct BindAddr(SocketAddr);

fn main() {
    let addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0.0.0.0:8080".to_string());
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
        .init_resource::<PlayerState>()
        .init_resource::<MapTiles>()
        .init_resource::<MapSettings>()
        .add_systems(Startup, (start_server, spawn_initial_state))
        .add_observer(handle_unit_action)
        .add_observer(handle_city_action)
        .add_observer(handle_finish_turn)
        .add_observer(handle_start_game)
        .add_observer(handle_set_map_config)
        .add_observer(claim_city_tiles)
        .add_observer(complete_unit_production)
        .add_systems(
            Update,
            (
                handle_new_clients,
                handle_disconnects,
                update_turn_phase,
                generate_map_on_start.run_if(should_generate_map),
                cleanup_map_on_lobby.run_if(should_cleanup_map),
                promote_waiting_players,
                recalculate_city_yields.run_if(any_city_yields_need_recalculation),
                // Resolution window: gated as a group so all resolvers see
                // a consistent "turn end" world; advance_turn closes the window.
                (
                    grow_city_population,
                    grant_city_gold,
                    resolve_ranged_attacks,
                    resolve_movement,
                    cleanup_dead_units,
                    regenerate_unattacked_cities,
                    resolve_fortify,
                    resolve_skip,
                    resolve_builds,
                    eliminate_defeated_players,
                    advance_city_production,
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
    let server = RenetServer::new(ConnectionConfig::from_channels(
        channels.server_configs(),
        channels.client_configs(),
    ));

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let socket = NativeSocket::new(UdpSocket::bind(addr.0)?)?;
    let server_config = ServerSetupConfig {
        current_time,
        max_clients: 8,
        protocol_id: PROTOCOL_ID,
        // renet2 supports multiple sockets per server; the outer Vec is indexed
        // by socket id, so socket 0 gets our single bind address.
        socket_addresses: vec![vec![addr.0]],
        authentication: ServerAuthentication::Unsecure,
    };
    let transport = NetcodeServerTransport::new(server_config, socket)?;

    commands.insert_resource(server);
    commands.insert_resource(transport);

    println!("Server listening on {}", addr.0);
    Ok(())
}

/// Spawns the lobby turn-state. The map itself is generated at game start
/// (`generate_map_on_start`), once the host's settings and player count are known.
fn spawn_initial_state(mut commands: Commands) {
    commands.spawn(TurnState {
        phase: TurnPhase::Lobby,
        turn_number: 0,
    });
    println!("Server ready; map will be generated when the host starts a game");
}
