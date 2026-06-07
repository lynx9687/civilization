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
    netcode::{
        BoxedSocket, NativeSocket, NetcodeServerTransport, ServerAuthentication, ServerSetupConfig,
        WebSocketAcceptor, WebSocketServer, WebSocketServerConfig,
    },
    renet2::{ConnectionConfig, RenetServer},
};
use shared::{components::*, map_settings::MapSettings, net, plugin::SharedPlugin};

use cities::*;
use cities_systems::*;
use combat::{cleanup_dead_units, resolve_movement, resolve_ranged_attacks};
use map_config::handle_set_map_config;
use map_gen::{
    MapTiles, cleanup_map_on_lobby, generate_map_on_start, should_cleanup_map, should_generate_map,
};
use players::{PlayerMap, handle_disconnects, handle_new_clients, promote_waiting_players};
use turn::*;

#[derive(Resource)]
struct BindAddr(SocketAddr);

fn main() {
    let addr: SocketAddr = match std::env::args().nth(1) {
        Some(s) => s.parse().expect("Invalid bind address"),
        None => net::DEFAULT_BIND_ADDR,
    };

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
        .init_resource::<TurnTimerState>()
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
                update_turn_timer,
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

    // Socket 0: native UDP for desktop clients.
    let udp_socket = NativeSocket::new(UdpSocket::bind(addr.0)?)?;

    // Socket 1: WebSocket for browser (wasm) clients, which can't speak UDP.
    // The WebSocket transport runs its accept/read/write loops on a tokio runtime;
    // we keep that runtime alive for the app's lifetime by storing it as a resource
    // (dropping it would shut down the worker threads and kill all WS connections).
    // It binds the same IP as UDP, on the shared WebSocket port.
    let ws_addr = SocketAddr::new(addr.0.ip(), net::GAME_WS_PORT);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Dev (default): no proxy, clients connect by IP, netcode encrypts the payload.
    // Prod (feature "wss"): a Caddy proxy terminates TLS and forwards plain ws to us.
    // `has_tls_proxy: true` tells netcode TLS already encrypts (so it won't double-
    // encrypt, matching the wss client), and the public address must be the dummy
    // 0.0.0.0:0 because clients connect by domain, not IP.
    #[cfg(not(feature = "wss"))]
    let (acceptor, ws_public_addr) = (
        WebSocketAcceptor::Plain {
            has_tls_proxy: false,
        },
        ws_addr,
    );
    #[cfg(feature = "wss")]
    let (acceptor, ws_public_addr) = (
        WebSocketAcceptor::Plain {
            has_tls_proxy: true,
        },
        SocketAddr::from(([0, 0, 0, 0], 0)),
    );

    let ws_socket = WebSocketServer::new(
        WebSocketServerConfig {
            acceptor,
            // Always bind the real local address; Caddy connects here over plain ws.
            listen: ws_addr,
            max_clients: net::MAX_CLIENTS,
        },
        runtime.handle().clone(),
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;

    let server_config = ServerSetupConfig {
        current_time,
        max_clients: net::MAX_CLIENTS,
        protocol_id: net::PROTOCOL_ID,
        // renet2 supports multiple sockets per server; the outer Vec is indexed
        // by socket id. Socket 0 is UDP, socket 1 is WebSocket. Clients pick the
        // matching socket id in their ClientAuthentication. socket_addresses[1]
        // must equal the client's `server_addr` for the WebSocket socket.
        socket_addresses: vec![vec![addr.0], vec![ws_public_addr]],
        authentication: ServerAuthentication::Unsecure,
    };
    let transport = NetcodeServerTransport::new_with_sockets(
        server_config,
        vec![BoxedSocket::new(udp_socket), BoxedSocket::new(ws_socket)],
    )?;

    commands.insert_resource(server);
    commands.insert_resource(transport);
    commands.insert_resource(WsRuntime(runtime));

    println!(
        "Server listening on UDP {} and WebSocket ws://{ws_addr}",
        addr.0
    );
    Ok(())
}

/// Holds the tokio runtime that drives the WebSocket server's background tasks.
/// Kept as a resource purely to keep the runtime (and its worker threads) alive
/// for the lifetime of the app.
#[derive(Resource)]
struct WsRuntime(#[allow(dead_code)] tokio::runtime::Runtime);

/// Spawns the lobby turn-state. The map itself is generated at game start
/// (`generate_map_on_start`), once the host's settings and player count are known.
fn spawn_initial_state(mut commands: Commands) {
    commands.spawn(TurnState {
        phase: TurnPhase::Lobby,
        turn_number: 0,
        turn_elapsed_secs: 0,
    });
    println!("Server ready; map will be generated when the host starts a game");
}
