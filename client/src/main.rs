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
use shared::{
    components::*,
    hex::{HexPosition, hex_to_pixel, pixel_to_hex},
    plugin::SharedPlugin,
};

const PROTOCOL_ID: u64 = 0;
const HEX_SIZE: f32 = 40.0;
const SQUARE_SIZE: f32 = 20.0;

#[derive(Resource)]
struct ServerAddr(SocketAddr);

/// Stores the local player's color index after receiving YourPlayer event.
#[derive(Resource)]
struct LocalPlayerColor(u8);

/// Tracks which turn the local player last submitted a move for.
#[derive(Resource, Default)]
struct LastSubmittedTurn(Option<u32>);

/// Tracks the currently hovered hex for highlighting.
#[derive(Resource, Default)]
struct HoveredHex(Option<HexPosition>);

/// Handles to shared hex materials for highlighting.
#[derive(Resource)]
struct HexMaterials {
    default: Handle<ColorMaterial>,
    hovered: Handle<ColorMaterial>,
    valid_move: Handle<ColorMaterial>,
}

#[derive(Component)]
struct TurnUiText;

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
        .add_systems(Startup, (setup_camera, connect_to_server, spawn_turn_ui))
        .add_observer(on_your_player)
        .add_systems(
            Update,
            (
                spawn_hex_visuals,
                spawn_player_visuals,
                update_player_positions,
                update_hex_highlights,
                handle_input,
                reset_submission_on_new_turn,
                update_turn_ui,
            ),
        )
        .run();
}

fn setup_camera(mut commands: Commands, mut materials: ResMut<Assets<ColorMaterial>>) {
    commands.spawn(Camera2d);

    let hex_materials = HexMaterials {
        default: materials.add(Color::srgb(0.15, 0.15, 0.2)),
        hovered: materials.add(Color::srgb(0.3, 0.3, 0.4)),
        valid_move: materials.add(Color::srgb(0.2, 0.4, 0.2)),
    };
    commands.insert_resource(hex_materials);
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

fn on_your_player(trigger: On<YourPlayer>, mut commands: Commands) {
    let color_index = trigger.color_index;
    commands.insert_resource(LocalPlayerColor(color_index));
    println!("Assigned player color index: {color_index}");
}

fn spawn_hex_visuals(
    tiles: Query<(Entity, &HexPosition), Added<HexTile>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    hex_materials: Res<HexMaterials>,
) {
    for (entity, pos) in &tiles {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(RegularPolygon::new(HEX_SIZE * 0.95, 6))),
            MeshMaterial2d(hex_materials.default.clone()),
            Transform::from_xyz(pixel.x, pixel.y, 0.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_6)),
        ));
    }
}

fn spawn_player_visuals(
    players: Query<(Entity, &Player, &HexPosition), Added<Player>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, player, pos) in &players {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let color = player_color(player.color_index);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Rectangle::new(SQUARE_SIZE, SQUARE_SIZE))),
            MeshMaterial2d(materials.add(color)),
            Transform::from_xyz(pixel.x, pixel.y, 1.0),
        ));
    }
}

#[allow(clippy::type_complexity)]
fn update_player_positions(
    mut players: Query<(&HexPosition, &mut Transform), (With<Player>, Changed<HexPosition>)>,
) {
    for (pos, mut transform) in &mut players {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        transform.translation.x = pixel.x;
        transform.translation.y = pixel.y;
    }
}

#[allow(clippy::too_many_arguments)]
fn update_hex_highlights(
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut tiles: Query<(&HexPosition, &mut MeshMaterial2d<ColorMaterial>), With<HexTile>>,
    hex_materials: Res<HexMaterials>,
    mut hovered: ResMut<HoveredHex>,
    local_color: Option<Res<LocalPlayerColor>>,
    players: Query<(&Player, &HexPosition), Without<HexTile>>,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
) {
    let cursor_hex = get_cursor_hex(&windows, &cameras);
    hovered.0 = cursor_hex;

    let valid_moves: Vec<HexPosition> = if let Some(ref local) = local_color {
        let can_move = turn_state
            .single()
            .is_ok_and(|s| s.phase == TurnPhase::Accepting)
            && !last_submitted
                .0
                .is_some_and(|t| turn_state.single().is_ok_and(|s| t >= s.turn_number));

        if can_move {
            players
                .iter()
                .find(|(p, _)| p.color_index == local.0)
                .map(|(_, pos)| pos.neighbors())
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    for (pos, mut material) in &mut tiles {
        if cursor_hex == Some(*pos) {
            *material = MeshMaterial2d(hex_materials.hovered.clone());
        } else if valid_moves.contains(pos) {
            *material = MeshMaterial2d(hex_materials.valid_move.clone());
        } else {
            *material = MeshMaterial2d(hex_materials.default.clone());
        }
    }
}

fn get_cursor_hex(
    windows: &Query<&Window>,
    cameras: &Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) -> Option<HexPosition> {
    let window = windows.single().ok()?;
    let (camera, transform) = cameras.single().ok()?;
    let cursor_pos = window.cursor_position()?;
    let world_pos = camera.viewport_to_world_2d(transform, cursor_pos).ok()?;
    Some(pixel_to_hex(world_pos, HEX_SIZE))
}

#[allow(clippy::too_many_arguments)]
fn handle_input(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    turn_state: Query<&TurnState>,
    mut last_submitted: ResMut<LastSubmittedTurn>,
    local_color: Option<Res<LocalPlayerColor>>,
    players: Query<(&Player, &HexPosition), Without<HexTile>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Some(ref local) = local_color else {
        return;
    };

    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    if last_submitted.0.is_some_and(|t| t >= state.turn_number) {
        return;
    }

    let Some(target) = get_cursor_hex(&windows, &cameras) else {
        return;
    };

    let Some((_, current_pos)) = players.iter().find(|(p, _)| p.color_index == local.0) else {
        return;
    };
    if !current_pos.is_neighbor(&target) {
        return;
    }

    commands.client_trigger(MoveAction { target });
    last_submitted.0 = Some(state.turn_number);
    println!("Submitted move to {:?}", target);
}

fn reset_submission_on_new_turn(
    turn_state: Query<&TurnState, Changed<TurnState>>,
    last_submitted: Res<LastSubmittedTurn>,
) {
    for state in &turn_state {
        if let Some(submitted) = last_submitted.0
            && state.turn_number > submitted
        {
            println!("New turn {}! Ready to move.", state.turn_number);
        }
    }
}

fn spawn_turn_ui(mut commands: Commands) {
    commands.spawn((
        TurnUiText,
        Text::new("Connecting..."),
        TextFont {
            font_size: 24.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
}

fn update_turn_ui(
    turn_state: Query<&TurnState>,
    local_color: Option<Res<LocalPlayerColor>>,
    last_submitted: Res<LastSubmittedTurn>,
    mut ui_text: Query<&mut Text, With<TurnUiText>>,
) {
    let Ok(mut text) = ui_text.single_mut() else {
        return;
    };

    if local_color.is_none() {
        **text = "Connecting...".to_string();
        return;
    }

    let Ok(state) = turn_state.single() else {
        **text = "Waiting for game state...".to_string();
        return;
    };

    let message = match state.phase {
        TurnPhase::WaitingForPlayers => "Waiting for players to join...".to_string(),
        TurnPhase::Accepting => {
            let submitted = last_submitted.0.is_some_and(|t| t >= state.turn_number);
            if submitted {
                format!("Turn {} -- Waiting for other players...", state.turn_number)
            } else {
                format!("Turn {} -- Click a neighbor hex to move", state.turn_number)
            }
        }
    };

    **text = message;
}
