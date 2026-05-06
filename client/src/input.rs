use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::unit_definition::{UnitRegistry, is_within_attack_range, is_within_move_range};
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

/// Tracks the local player id and other permanent identity info.
#[derive(Resource, Default)]
pub struct Controller {
    pub player_entity: Option<Entity>,
}

/// Selection / targeting state. Drives the action bar visibility and
/// the map-highlight overlay. Idle = no unit selected.
#[derive(Resource, Default)]
pub enum UiState {
    #[default]
    Idle,
    UnitSelected {
        unit: Entity,
    },
    Targeting {
        unit: Entity,
        verb: TargetableVerb,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum TargetableVerb {
    Move,
    Attack,
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
    ui_state: Res<UiState>,
    units: Query<(&Unit, &HexPosition, &Owner)>,
    registry: Res<UnitRegistry>,
    all_tiles: Query<&HexPosition, With<HexTile>>,
    controller: Res<Controller>,
) {
    let cursor_hex = get_cursor_hex(&cursor);
    hovered.0 = cursor_hex;

    let Some(player_entity) = controller.player_entity else {
        return;
    };

    // compute the current overlay set based on UiState
    let (move_targets, attack_targets): (Vec<HexPosition>, Vec<HexPosition>) = match *ui_state {
        UiState::Targeting { unit, verb } => 'overlay: {
            let Ok((u, pos, _)) = units.get(unit) else {
                // stale unit ref — fall through with no overlay so the loop repaints to default
                break 'overlay (Vec::new(), Vec::new());
            };
            let Some(def) = registry.get(&u.type_id) else {
                break 'overlay (Vec::new(), Vec::new());
            };
            match verb {
                TargetableVerb::Move => {
                    let moves = all_tiles
                        .iter()
                        .filter(|t| is_within_move_range(pos, t, def.move_budget))
                        .copied()
                        .collect();
                    (moves, Vec::new())
                }
                TargetableVerb::Attack => {
                    // only enemy-occupied hexes within range light up
                    let attacks = units
                        .iter()
                        .filter_map(|(_, p, owner)| {
                            let is_enemy = owner.0 != player_entity;
                            if is_enemy && is_within_attack_range(pos, p, def.attack_range) {
                                Some(*p)
                            } else {
                                None
                            }
                        })
                        .collect();
                    (Vec::new(), attacks)
                }
            }
        }
        _ => (Vec::new(), Vec::new()),
    };

    for (pos, mut material) in &mut tiles {
        if cursor_hex == Some(*pos) {
            *material = MeshMaterial2d(hex_materials.hovered.clone());
        } else if attack_targets.contains(pos) {
            *material = MeshMaterial2d(hex_materials.valid_attack.clone());
        } else if move_targets.contains(pos) {
            *material = MeshMaterial2d(hex_materials.valid_move.clone());
        } else {
            *material = MeshMaterial2d(hex_materials.default.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_left_click(
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: CursorWorld,
    mut commands: Commands,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    controller: Res<Controller>,
    mut ui_state: ResMut<UiState>,
    units: Query<(Entity, &Unit, &Owner, &HexPosition)>,
    registry: Res<UnitRegistry>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
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
    let Some(player_entity) = controller.player_entity else {
        return;
    };

    // is the click on one of my owned units?
    let owned_unit_at = |hex: HexPosition| -> Option<Entity> {
        for (entity, _unit, owner, pos) in &units {
            if owner.0 == player_entity && *pos == hex {
                return Some(entity);
            }
        }
        None
    };

    match *ui_state {
        UiState::Idle => {
            if let Some(entity) = owned_unit_at(target) {
                *ui_state = UiState::UnitSelected { unit: entity };
            }
        }
        UiState::UnitSelected { unit: _ } => {
            if let Some(entity) = owned_unit_at(target) {
                *ui_state = UiState::UnitSelected { unit: entity };
            } else {
                *ui_state = UiState::Idle;
            }
        }
        UiState::Targeting { unit, verb } => {
            // clicking another owned unit always switches selection
            if let Some(entity) = owned_unit_at(target) {
                *ui_state = UiState::UnitSelected { unit: entity };
                return;
            }
            let Ok((_, u, _, pos)) = units.get(unit) else {
                *ui_state = UiState::Idle;
                return;
            };
            let Some(def) = registry.get(&u.type_id) else {
                *ui_state = UiState::Idle;
                return;
            };
            match verb {
                TargetableVerb::Move => {
                    if is_within_move_range(pos, &target, def.move_budget) {
                        commands.client_trigger(UnitActionEvent {
                            unit,
                            action: UnitAction::Move { target },
                        });
                        *ui_state = UiState::Idle;
                    } else {
                        // invalid hex → fall back to selection state, bar stays
                        *ui_state = UiState::UnitSelected { unit };
                    }
                }
                TargetableVerb::Attack => {
                    // attacker is at `pos`; enemies are units with a different owner_id at `target`
                    let enemy_here = units
                        .iter()
                        .any(|(_, _, owner, p)| *p == target && owner.0 != player_entity);
                    if is_within_attack_range(pos, &target, def.attack_range) && enemy_here {
                        commands.client_trigger(UnitActionEvent {
                            unit,
                            action: UnitAction::Attack { target },
                        });
                        *ui_state = UiState::Idle;
                    } else {
                        *ui_state = UiState::UnitSelected { unit };
                    }
                }
            }
        }
    }
}

pub fn handle_escape_key(keys: Res<ButtonInput<KeyCode>>, mut ui_state: ResMut<UiState>) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    *ui_state = match *ui_state {
        UiState::Targeting { unit, .. } => UiState::UnitSelected { unit },
        _ => UiState::Idle,
    };
}

// drops UiState back to Idle if the unit it references no longer exists
pub fn prune_stale_selection(mut ui_state: ResMut<UiState>, units: Query<(), With<Unit>>) {
    let referenced = match *ui_state {
        UiState::Idle => return,
        UiState::UnitSelected { unit } => unit,
        UiState::Targeting { unit, .. } => unit,
    };
    if units.get(referenced).is_err() {
        *ui_state = UiState::Idle;
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
