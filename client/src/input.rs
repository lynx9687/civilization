use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::unit_definition::{UnitRegistry, is_within_move_range};
use shared::{
    components::*,
    events::*,
    hex::{HexPosition, pixel_to_hex},
    units::*,
};

use crate::HEX_SIZE;
use crate::visuals::HexMaterials;

/// Tracks which turn the local player last submitted a move for.
#[derive(Resource, Default)]
pub struct LastSubmittedTurn(pub Option<u32>);

/// Tracks the currently hovered hex for highlighting.
#[derive(Resource, Default)]
pub struct HoveredHex(Option<HexPosition>);

/// Trakcks the currently selected unit
/// and other information related to controling game
#[derive(Resource, Default)]
pub struct Controller {
    pub player_id: Option<u32>,
    pub selected_unit: Option<Entity>,
}

#[derive(SystemParam)]
pub struct CursorWorld<'w, 's> {
    windows: Query<'w, 's, &'static Window>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<Camera2d>>,
}

fn get_cursor_hex(cursor: &CursorWorld) -> Option<HexPosition> {
    let window = cursor.windows.single().ok()?;
    let (camera, transform) = cursor.cameras.single().ok()?;
    let cursor_pos = window.cursor_position()?;
    let world_pos = camera.viewport_to_world_2d(transform, cursor_pos).ok()?;
    Some(pixel_to_hex(world_pos, HEX_SIZE))
}

#[allow(clippy::too_many_arguments)]
pub fn update_hex_highlights(
    cursor: CursorWorld,
    mut tiles: Query<(&HexPosition, &mut MeshMaterial2d<ColorMaterial>), With<HexTile>>,
    hex_materials: Res<HexMaterials>,
    mut hovered: ResMut<HoveredHex>,
    controller: ResMut<Controller>,
    units: Query<(&Unit, &HexPosition)>,
    registry: Res<UnitRegistry>,
    all_tiles: Query<&HexPosition, With<HexTile>>,
) {
    let cursor_hex = get_cursor_hex(&cursor);
    hovered.0 = cursor_hex;

    let valid_moves: Vec<HexPosition> = if let Some(selected_unit) = controller.selected_unit
        && let Ok((unit, pos)) = units.get(selected_unit)
        && let Some(def) = registry.get(&unit.type_name)
    {
        all_tiles
            .iter()
            .filter(|tile_pos| is_within_move_range(pos, tile_pos, def.move_budget))
            .copied()
            .collect()
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

pub fn handle_left_click(
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: CursorWorld,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    mut controller: ResMut<Controller>,
    units: Query<(Entity, &Owner, &HexPosition), With<Unit>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    //check whether turn is active
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }
    if last_submitted.0.is_some_and(|t| t >= state.turn_number) {
        return;
    }

    let Some(target) = get_cursor_hex(&cursor) else {
        return;
    };

    let Some(player_id) = controller.player_id else {
        return;
    };

    println!("Target {target:?}");
    println!("Player entity {player_id}");
    // select clicked unit
    for (unit_entity, owner, pos) in units {
        let x = owner.player_id;
        println!("Unit {unit_entity} with owner {x} at position {pos:?}");
        if owner.player_id == player_id && *pos == target {
            controller.selected_unit = Some(unit_entity);
            println!("Selected unit {unit_entity}");
            return;
        }
    }
    controller.selected_unit = None;
    println!("Deselected unit");
}

#[allow(clippy::too_many_arguments)]
pub fn handle_right_click(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: CursorWorld,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    mut controller: ResMut<Controller>,
    units: Query<(&HexPosition, &Unit)>,
    registry: Res<UnitRegistry>,
) {
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    //check whether turn is active
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }
    if last_submitted.0.is_some_and(|t| t >= state.turn_number) {
        return;
    }

    let Some(target) = get_cursor_hex(&cursor) else {
        return;
    };

    // proceed only if unit is currently selected
    let Some(unit_entity) = controller.selected_unit else {
        return;
    };

    let Ok((unit_pos, unit)) = units.get(unit_entity) else {
        return;
    };

    let Some(def) = registry.get(&unit.type_name) else {
        return;
    };

    if is_within_move_range(unit_pos, &target, def.move_budget) {
        commands.client_trigger(MoveAction {
            unit_id: unit.id,
            target,
        });
        //deselect unit
        controller.selected_unit = None;
    }
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
