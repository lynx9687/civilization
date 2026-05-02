use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::{
    cities::City,
    components::*,
    events::*,
    hex::{HexPosition, pixel_to_hex},
    tiles::TileOwner,
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
    pub selected_city: Option<Entity>,
}

#[derive(SystemParam)]
pub struct CursorWorld<'w, 's> {
    windows: Query<'w, 's, &'static Window>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<Camera2d>>,
}

type TileHighlightQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static HexPosition,
        Option<&'static TileOwner>,
        &'static mut MeshMaterial2d<ColorMaterial>,
    ),
    With<HexTile>,
>;

fn get_cursor_hex(cursor: &CursorWorld) -> Option<HexPosition> {
    let window = cursor.windows.single().ok()?;
    let (camera, transform) = cursor.cameras.single().ok()?;
    let cursor_pos = window.cursor_position()?;
    let world_pos = camera.viewport_to_world_2d(transform, cursor_pos).ok()?;
    Some(pixel_to_hex(world_pos, HEX_SIZE))
}

pub fn update_hex_highlights(
    cursor: CursorWorld,
    mut tiles: TileHighlightQuery,
    hex_materials: Res<HexMaterials>,
    mut hovered: ResMut<HoveredHex>,
    controller: ResMut<Controller>,
    units: Query<&HexPosition, With<Unit>>,
    players: Query<&Player>,
) {
    let cursor_hex = get_cursor_hex(&cursor);
    hovered.0 = cursor_hex;

    let valid_moves: Vec<HexPosition> = if let Some(selected_unit) = controller.selected_unit {
        if let Ok(pos) = units.get(selected_unit) {
            pos.neighbors()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    for (pos, owner, mut material) in &mut tiles {
        if cursor_hex == Some(*pos) {
            *material = MeshMaterial2d(hex_materials.hovered.clone());
        } else if valid_moves.contains(pos) {
            *material = MeshMaterial2d(hex_materials.valid_move.clone());
        } else if let Some(tile_owner) = owner {
            let color_index = players
                .iter()
                .find(|player| player.player_id == tile_owner.player_id)
                .map_or(0, |player| player.color_index);
            *material = MeshMaterial2d(hex_materials.claimed[color_index as usize].clone());
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
    cities: Query<(Entity, &HexPosition), With<City>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
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

    if let Ok(state) = turn_state.single()
        && state.phase == TurnPhase::Accepting
        && last_submitted.0.is_none_or(|t| t < state.turn_number)
    {
        // select clicked unit
        for (unit_entity, owner, pos) in units {
            let x = owner.player_id;
            println!("Unit {unit_entity} with owner {x} at position {pos:?}");
            if owner.player_id == player_id && *pos == target {
                controller.selected_unit = Some(unit_entity);
                controller.selected_city = None;
                println!("Selected unit {unit_entity}");
                return;
            }
        }
    }

    for (city_entity, pos) in cities {
        if *pos == target {
            controller.selected_unit = None;
            controller.selected_city = Some(city_entity);
            println!("Selected city {city_entity}");
            return;
        }
    }

    controller.selected_unit = None;
    controller.selected_city = None;
    println!("Deselected unit/city");
}

pub fn handle_right_click(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: CursorWorld,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    mut controller: ResMut<Controller>,
    units: Query<(&HexPosition, &Unit)>,
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

    if target.is_neighbor(unit_pos) {
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
