mod input;
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
use shared::{events::*, plugin::SharedPlugin};

use input::*;
use ui::*;
use visuals::*;

const PROTOCOL_ID: u64 = 0;
const HEX_SIZE: f32 = 40.0;

#[derive(Resource)]
struct ServerAddr(SocketAddr);

/// Stores the local player's color index after receiving YourPlayer event.
#[derive(Resource)]
pub struct LocalPlayerColor(pub u8);

fn main() {
    let addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:5000".to_string());
    let addr: SocketAddr = addr_str.parse().expect("Invalid server address");

    println!("Connecting to server at {addr}");

    App::new()
        .add_plugins((
            DefaultPlugins,
            RepliconPlugins,
            RepliconRenetPlugins,
            SharedPlugin,
        ))
        .insert_resource(ServerAddr(addr))
        .init_resource::<LastSubmittedTurn>()
        .init_resource::<HoveredHex>()
        .init_resource::<Controller>()
        .init_resource::<UiState>()
        .init_state::<PlayerTurnPhase>()
        .add_systems(Startup, (setup_camera, connect_to_server, spawn_turn_ui))
        .add_observer(on_your_player)
        .add_observer(handle_verb_button_click)
        .add_systems(
            Update,
            (
                spawn_hex_visuals,
                spawn_unit_visuals,
                update_unit_positions,
                update_hex_highlights,
                handle_left_click,
                handle_escape_key,
                prune_stale_selection,
                reset_submission_on_new_turn,
                update_turn_ui,
                update_action_bar,
                finish_turn_trigger_system.run_if(in_state(PlayerTurnPhase::Input)),
                finish_turn_visual_system,
                reset_player_turn_phase,
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

fn on_your_player(
    trigger: On<YourPlayer>,
    mut commands: Commands,
    mut controller: ResMut<Controller>,
) {
    let color_index = trigger.color_index;
    commands.insert_resource(LocalPlayerColor(color_index));
    println!("Assigned player color index: {color_index}");
    let player_entity = trigger.player_entity;
    println!("Received player_entity: {player_entity}");
    controller.player_entity = Some(player_entity);
}
