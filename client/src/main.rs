mod audio;
mod camera;
mod input;
mod lobby;
mod ui;
mod visuals;

use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    RenetChannelsExt, RenetClient, RepliconRenetPlugins,
    netcode::{ClientAuthentication, NetcodeClientTransport},
    renet::ConnectionConfig,
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
struct ServerAddr(SocketAddr);

fn main() {
    let addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "158.180.62.178:5000".to_string());
    let addr: SocketAddr = addr_str.parse().expect("Invalid server address");

    println!("Connecting to server at {addr}");
    let asset_path = assets_dir();

    App::new()
        .add_plugins((
            DefaultPlugins.set(AssetPlugin {
                file_path: asset_path.to_string_lossy().into_owned(),
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

fn connect_to_server(
    mut commands: Commands,
    channels: Res<RepliconChannels>,
    addr: Res<ServerAddr>,
) -> Result<()> {
    let server_channels_config = channels.server_configs();
    let client_channels_config = channels.client_configs();

    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let client_id = current_time.as_millis() as u64;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr: addr.0,
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
