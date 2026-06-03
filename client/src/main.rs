mod audio;
mod camera;
mod input;
mod lobby;
mod ui;
mod visuals;

use std::net::SocketAddr;
// web-time re-exports std on native; on wasm it uses the browser clock, since
// std::time::SystemTime::now() is unimplemented (and panics) there.
use web_time::SystemTime;

// UDP is only used by the native transport; the wasm build connects over WebSocket.
#[cfg(not(target_arch = "wasm32"))]
use std::net::{Ipv4Addr, UdpSocket};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet2::{
    RenetChannelsExt, RepliconRenetPlugins,
    netcode::{ClientAuthentication, ClientSocket, NetcodeClientTransport},
    renet2::{ConnectionConfig, RenetClient},
};
use shared::{assets::assets_dir, events::*, map_settings::MapSettings, plugin::SharedPlugin};

use audio::*;
use camera::*;
use input::*;
use lobby::*;
use ui::*;
use visuals::*;

const PROTOCOL_ID: u64 = 0;
const HEX_SIZE: f32 = 40.0;

#[derive(Resource)]
// On wasm the WebSocket endpoint is currently hardcoded, so this field isn't read
// there yet; native still uses it. Remove once wasm derives its address from here.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct ServerAddr(SocketAddr);

fn main() {
    let addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "158.180.62.178:8080".to_string());
    let addr: SocketAddr = addr_str.parse().expect("Invalid server address");

    println!("Connecting to server at {addr}");
    let asset_path = assets_dir();

    App::new()
        .add_plugins((
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_path.to_string_lossy().into_owned(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        // On web, attach to our own canvas and track its parent's
                        // size so the game fills the page. Both fields are no-ops on
                        // native, so this is safe to set unconditionally.
                        canvas: Some("#game-canvas".into()),
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                }),
            RepliconPlugins,
            RepliconRenetPlugins,
            SharedPlugin,
        ))
        .insert_resource(ServerAddr(addr))
        .init_resource::<LastSubmittedTurn>()
        .init_resource::<HoveredHex>()
        .init_resource::<Controller>()
        .init_resource::<UiState>()
        .init_resource::<CameraZoom>()
        // Host's pending lobby map choice; sent to the server via SetMapConfig.
        .init_resource::<MapSettings>()
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_hex_materials,
                connect_to_server,
                spawn_turn_ui,
                spawn_lobby_ui,
                play_background_music,
            ),
        )
        .add_observer(on_your_player)
        .add_observer(finish_turn_clicked)
        .add_observer(handle_verb_button_click)
        .add_observer(handle_production_button_click)
        .add_observer(handle_start_game_click)
        .add_observer(handle_map_config_click)
        .add_systems(
            Update,
            (
                (
                    spawn_hex_visuals,
                    spawn_unit_visuals,
                    update_unit_colors,
                    spawn_city_visuals,
                    update_city_visuals,
                    update_unit_positions,
                    update_unit_health_bars,
                    update_city_health_bars,
                ),
                (
                    move_camera_with_keyboard,
                    zoom_camera_with_scroll,
                    update_hex_highlights,
                    handle_left_click,
                    handle_right_click,
                    handle_escape_key,
                    prune_stale_selection,
                ),
                (
                    reset_submission_on_new_turn,
                    populate_production_bar,
                    update_turn_ui,
                    update_city_ui,
                    update_action_bar,
                    update_production_bar,
                    update_lobby_ui,
                    update_map_config_ui,
                    update_lose_screen,
                ),
            ),
        )
        .run();
}

#[cfg(not(target_arch = "wasm32"))]
use bevy_replicon_renet2::netcode::NativeSocket;

#[cfg(not(target_arch = "wasm32"))]
fn connect_to_server(
    mut commands: Commands,
    channels: Res<RepliconChannels>,
    addr: Res<ServerAddr>,
) -> Result<()> {
    let socket = NativeSocket::new(UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?)?;
    let server_channels_config = channels.server_configs();
    let client_channels_config = channels.client_configs();

    let client = RenetClient::new(
        ConnectionConfig::from_channels(server_channels_config, client_channels_config),
        socket.is_reliable(),
    );

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let client_id = current_time.as_millis() as u64;
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        socket_id: 0,
        protocol_id: PROTOCOL_ID,
        server_addr: addr.0,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(current_time, authentication, socket)?;

    commands.insert_resource(client);
    commands.insert_resource(transport);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
use bevy_replicon_renet2::netcode::{WebSocketClient, WebSocketClientConfig};

#[cfg(target_arch = "wasm32")]
fn connect_to_server(
    mut commands: Commands,
    channels: Res<RepliconChannels>,
    // ServerAddr holds the deployment (UDP) address; the local WebSocket endpoint is
    // hardcoded below for now, so the resource is intentionally unused here.
    _addr: Res<ServerAddr>,
) -> Result<()> {
    // The URL and `server_addr` must agree: WebSocketClient::send rejects any packet
    // whose destination differs from the address derived from this URL, so we build
    // both from one SocketAddr. (Localhost WebSocket is the server's UDP port + 1.)
    let ws_addr: SocketAddr = "127.0.0.1:8081".parse().unwrap();
    let server_url = url::Url::parse(&format!("ws://{ws_addr}"))?;
    let socket = WebSocketClient::new(WebSocketClientConfig { server_url })
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let server_channels_config = channels.server_configs();
    let client_channels_config = channels.client_configs();

    let client = RenetClient::new(
        ConnectionConfig::from_channels(server_channels_config, client_channels_config),
        socket.is_reliable(),
    );

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let client_id = current_time.as_millis() as u64;
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        // Socket 1 on the server is the WebSocket transport (socket 0 is UDP).
        socket_id: 1,
        protocol_id: PROTOCOL_ID,
        server_addr: ws_addr,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(current_time, authentication, socket)?;

    commands.insert_resource(client);
    commands.insert_resource(transport);
    Ok(())
}

fn on_your_player(trigger: On<YourPlayer>, mut controller: ResMut<Controller>) {
    controller.player_entity = Some(trigger.player_entity);
    println!("Received player_entity: {}", trigger.player_entity);
}
