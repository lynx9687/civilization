use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::{
    components::*,
    hex::{HexPosition, pixel_to_hex},
};

use crate::HEX_SIZE;
use crate::LocalPlayerColor;
use crate::visuals::HexMaterials;

/// Tracks which turn the local player last submitted a move for.
#[derive(Resource, Default)]
pub struct LastSubmittedTurn(pub Option<u32>);

/// Tracks the currently hovered hex for highlighting.
#[derive(Resource, Default)]
pub struct HoveredHex(Option<HexPosition>);

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
pub fn update_hex_highlights(
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

#[allow(clippy::too_many_arguments)]
pub fn handle_input(
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

pub fn reset_submission_on_new_turn(
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
