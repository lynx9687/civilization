use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::unit_definition::{UnitRegistry, is_within_attack_range, is_within_move_range};
use shared::{
    cities::{City, CityOwner},
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

/// Tracks the local player id and other permanent identity info.
#[derive(Resource, Default)]
pub struct Controller {
    pub player_entity: Option<Entity>,
}

pub fn local_player_defeated(
    controller: &Controller,
    defeated: &Query<(), With<DefeatedPlayer>>,
) -> bool {
    controller
        .player_entity
        .is_some_and(|player| defeated.contains(player))
}

pub fn local_player_victorious(
    controller: &Controller,
    victorious: &Query<(), With<VictoriousPlayer>>,
) -> bool {
    controller
        .player_entity
        .is_some_and(|player| victorious.contains(player))
}

/// Terminal outcome for the local player once the server marks the game complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOutcome {
    Won,
    Lost,
}

/// Selection / targeting state. Drives the action bar visibility and
/// the map-highlight overlay. Idle = no unit selected.
#[derive(Resource, Default, PartialEq, Eq)]
pub enum UiState {
    #[default]
    Idle,
    UnitSelected {
        unit: Entity,
    },
    CitySelected {
        city: Entity,
    },
    Targeting {
        unit: Entity,
        verb: TargetableVerb,
    },
    GameFinished {
        outcome: GameOutcome,
    },
}

impl UiState {
    pub fn is_game_finished(&self) -> bool {
        matches!(self, UiState::GameFinished { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetableVerb {
    Move,
    Attack,
}

pub fn sync_game_finished_ui_state(
    controller: Res<Controller>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
    mut ui_state: ResMut<UiState>,
) {
    let outcome = if local_player_victorious(&controller, &victorious) {
        Some(GameOutcome::Won)
    } else if local_player_defeated(&controller, &defeated) {
        Some(GameOutcome::Lost)
    } else {
        None
    };

    let Some(outcome) = outcome else {
        return;
    };

    let game_finished = UiState::GameFinished { outcome };
    if *ui_state != game_finished {
        *ui_state = game_finished;
    }
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

#[allow(clippy::too_many_arguments)]
pub fn update_hex_highlights(
    cursor: CursorWorld,
    mut tiles: TileHighlightQuery,
    hex_materials: Res<HexMaterials>,
    mut hovered: ResMut<HoveredHex>,
    ui_state: Res<UiState>,
    units: Query<(&Unit, &HexPosition, &Owner)>,
    cities: Query<(&HexPosition, &CityOwner), With<City>>,
    registry: Res<UnitRegistry>,
    all_tiles: Query<&HexPosition, With<HexTile>>,
    controller: Res<Controller>,
    players: Query<&Player>,
) {
    let cursor_hex = get_cursor_hex(&cursor);
    hovered.0 = cursor_hex;

    let player_entity = controller.player_entity;

    // compute the current overlay set based on UiState
    let (move_targets, attack_targets): (Vec<HexPosition>, Vec<HexPosition>) = match *ui_state {
        UiState::Targeting { unit, verb } => 'overlay: {
            let Some(player_entity) = player_entity else {
                break 'overlay (Vec::new(), Vec::new());
            };
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
                        .filter(|t| {
                            cities
                                .iter()
                                .find(|(city_pos, _)| city_pos == t)
                                .is_none_or(|(_, city_owner)| {
                                    city_owner.entity == player_entity || def.attack_range == 1
                                })
                        })
                        .copied()
                        .collect();
                    (moves, Vec::new())
                }
                TargetableVerb::Attack => {
                    // only enemy-occupied hexes within range light up
                    let mut attacks = units
                        .iter()
                        .filter_map(|(_, p, owner)| {
                            let is_enemy = owner.0 != player_entity;
                            if is_enemy && is_within_attack_range(pos, p, def.attack_range) {
                                Some(*p)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    attacks.extend(cities.iter().filter_map(|(p, owner)| {
                        let is_enemy = owner.entity != player_entity;
                        if is_enemy && is_within_attack_range(pos, p, def.attack_range) {
                            Some(*p)
                        } else {
                            None
                        }
                    }));
                    (Vec::new(), attacks)
                }
            }
        }
        _ => (Vec::new(), Vec::new()),
    };

    for (pos, owner, mut material) in &mut tiles {
        if cursor_hex == Some(*pos) {
            *material = MeshMaterial2d(hex_materials.hovered.clone());
        } else if attack_targets.contains(pos) {
            *material = MeshMaterial2d(hex_materials.valid_attack.clone());
        } else if move_targets.contains(pos) {
            *material = MeshMaterial2d(hex_materials.valid_move.clone());
        } else if let Some(tile_owning_player) = owner.and_then(|x| x.player_entity) {
            let Ok(owning_player) = players.get(tile_owning_player) else {
                continue;
            };
            *material =
                MeshMaterial2d(hex_materials.claimed[owning_player.color_index as usize].clone());
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
    cities: Query<(&HexPosition, &CityOwner), With<City>>,
    registry: Res<UnitRegistry>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    if ui_state.is_game_finished() {
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
        UiState::Idle | UiState::CitySelected { .. } => {
            if let Some(entity) = owned_unit_at(target) {
                *ui_state = UiState::UnitSelected { unit: entity };
            } else {
                *ui_state = UiState::Idle;
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
                    let city_at_target = cities.iter().find(|(city_pos, _)| **city_pos == target);
                    let valid_city_target = city_at_target.is_none_or(|(_, city_owner)| {
                        city_owner.entity == player_entity || def.attack_range == 1
                    });
                    if is_within_move_range(pos, &target, def.move_budget) && valid_city_target {
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
                        .any(|(_, _, owner, p)| *p == target && owner.0 != player_entity)
                        || cities
                            .iter()
                            .any(|(p, owner)| *p == target && owner.entity != player_entity);
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
        UiState::GameFinished { .. } => {}
    }
}

/// Allows selecting both unit/city when they are on the same tile. This is a temporary solution
/// Better handling of user input / gui should be considered in the future
pub fn handle_right_click(
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: CursorWorld,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    mut ui_state: ResMut<UiState>,
    cities: Query<(Entity, &HexPosition), With<City>>,
) {
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    if ui_state.is_game_finished() {
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

    // handle clicking city
    for (city_entity, pos) in cities {
        if *pos == target {
            *ui_state = UiState::CitySelected { city: city_entity };
            println!("Selected city {city_entity}");
            return;
        }
    }
}

pub fn handle_escape_key(keys: Res<ButtonInput<KeyCode>>, mut ui_state: ResMut<UiState>) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    *ui_state = match *ui_state {
        UiState::Targeting { unit, .. } => UiState::UnitSelected { unit },
        UiState::GameFinished { outcome } => UiState::GameFinished { outcome },
        _ => UiState::Idle,
    };
}

// drops UiState back to Idle if the entity it references no longer exists
pub fn prune_stale_selection(
    mut ui_state: ResMut<UiState>,
    units: Query<(), With<Unit>>,
    cities: Query<(), With<City>>,
) {
    let stale = match *ui_state {
        UiState::Idle => return,
        UiState::UnitSelected { unit } | UiState::Targeting { unit, .. } => {
            units.get(unit).is_err()
        }
        UiState::CitySelected { city } => cities.get(city).is_err(),
        UiState::GameFinished { .. } => return,
    };
    if stale {
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
