use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::unit_definition::{
    UnitRegistry, is_reachable, is_within_attack_range, reachable_tiles,
};
use shared::{
    cities::{City, CityOwner},
    components::*,
    events::*,
    hex::{HexPosition, pixel_to_hex},
    terrain::Terrain,
    tiles::TileOwner,
    units::*,
};

use crate::HEX_SIZE;
use crate::visuals::{HexMaterials, HoverHighlightVisual, hex_mesh};

const HOVER_HIGHLIGHT_Z: f32 = 0.2;
const TARGET_DOT_Z: f32 = 3.0;

/// Tracks which turn the local player last submitted a move for.
#[derive(Resource, Default)]
pub struct LastSubmittedTurn(pub Option<u32>);

/// Tracks the currently hovered hex for highlighting.
#[derive(Resource, Default)]
pub struct HoveredHex(Option<HexPosition>);

impl HoveredHex {
    pub fn current(&self) -> Option<HexPosition> {
        self.0
    }
}

/// Tracks the local player id and other permanent identity info.
#[derive(Resource, Default)]
pub struct Controller {
    pub player_entity: Option<Entity>,
    pub selected_city: Option<Entity>,
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

pub fn local_player_game_over(
    controller: &Controller,
    defeated: &Query<(), With<DefeatedPlayer>>,
    victorious: &Query<(), With<VictoriousPlayer>>,
) -> bool {
    local_player_defeated(controller, defeated) || local_player_victorious(controller, victorious)
}

/// Selection / targeting state and turn UI mode.
/// Idle / selection is available while the local player can act.
/// Locked is used once the player has finished their turn.
#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum UiState {
    Input {
        selection: InputSelection,
    },
    #[default]
    Locked,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InputSelection {
    Idle,
    UnitSelected { unit: Entity },
    Targeting { unit: Entity, verb: TargetableVerb },
}

impl UiState {
    pub fn selection(&self) -> Option<&InputSelection> {
        match self {
            UiState::Input { selection } => Some(selection),
            UiState::Locked => None,
        }
    }

    pub fn is_locked(&self) -> bool {
        matches!(self, UiState::Locked)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Copy, Debug)]
pub enum TargetableVerb {
    Move,
    Attack,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetDotType {
    Move,
    Attack,
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
        Entity,
        &'static HexPosition,
        Option<&'static TileOwner>,
        Option<&'static Terrain>,
        Option<&'static Children>,
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

#[derive(SystemParam)]
pub struct MeshAssets<'w, 's> {
    pub assets: ResMut<'w, Assets<Mesh>>,
    pub hover_highlights: Query<'w, 's, (), With<HoverHighlightVisual>>,
    pub dot: Local<'s, Option<Handle<Mesh>>>,
    pub hover: Local<'s, Option<Handle<Mesh>>>,
}

#[allow(clippy::too_many_arguments)]
pub fn update_hex_highlights(
    cursor: CursorWorld,
    mut commands: Commands,
    mut tiles: TileHighlightQuery,
    mut mesh_bundle: MeshAssets,
    hex_materials: Res<HexMaterials>,
    mut hovered: ResMut<HoveredHex>,
    ui_state: Res<State<UiState>>,
    units: Query<(&Unit, &HexPosition, &Owner)>,
    cities: Query<(&HexPosition, &CityOwner), With<City>>,
    registry: Res<UnitRegistry>,
    all_tiles: Query<&HexPosition, With<HexTile>>,
    terrain_tiles: Query<(&HexPosition, &Terrain), With<HexTile>>,
    controller: Res<Controller>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
    dot_type_query: Query<&TargetDotType>,
) {
    let (meshes, hover_highlights, dot_mesh, hover_mesh) = (
        &mut mesh_bundle.assets,
        &mesh_bundle.hover_highlights,
        &mut mesh_bundle.dot,
        &mut mesh_bundle.hover,
    );
    let cursor_hex = get_cursor_hex(&cursor);
    hovered.0 = cursor_hex;

    let player_entity = controller.player_entity;
    let is_game_over = local_player_game_over(&controller, &defeated, &victorious);

    // compute the current overlay set based on UiState
    let (move_targets, attack_targets): (Vec<HexPosition>, Vec<HexPosition>) = if is_game_over {
        (Vec::new(), Vec::new())
    } else {
        match ui_state.selection() {
            Some(InputSelection::Targeting { unit, verb }) => 'overlay: {
                let Some(player_entity) = player_entity else {
                    break 'overlay (Vec::new(), Vec::new());
                };
                let Ok((u, pos, _)) = units.get(*unit) else {
                    // stale unit ref — fall through with no overlay so the loop repaints to default
                    break 'overlay (Vec::new(), Vec::new());
                };
                let Some(def) = registry.get(&u.type_id) else {
                    break 'overlay (Vec::new(), Vec::new());
                };
                match verb {
                    TargetableVerb::Move => {
                        // Compute the terrain-aware reachable set ONCE (same shared
                        // fn the server validates with, so the preview can't lie),
                        // then membership-filter the tiles instead of re-pathing per
                        // tile. The terrain map doubles as the on-map tile set.
                        let terrain_map: std::collections::HashMap<HexPosition, Terrain> =
                            terrain_tiles.iter().map(|(p, t)| (*p, *t)).collect();
                        let reachable = reachable_tiles(pos, def, |p| terrain_map.get(p).copied());
                        let moves = all_tiles
                            .iter()
                            .filter(|t| reachable.contains_key(*t))
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
        }
    };

    for (entity, pos, _owner, terrain, children, mut material) in &mut tiles {
        // Always keep the base terrain material on the tile. Hover is drawn as a
        // child overlay so terrain textures and ownership visuals stay intact.
        let base = terrain
            .map(|t| hex_materials.terrain_material(*t))
            .unwrap_or_else(|| hex_materials.default.clone());
        *material = MeshMaterial2d(base);

        let existing_hover = children.and_then(|children| {
            children
                .iter()
                .find(|child| hover_highlights.get(*child).is_ok())
        });

        if cursor_hex == Some(*pos) {
            if existing_hover.is_none() {
                commands.entity(entity).with_children(|parent| {
                    let mesh_handle = hover_mesh
                        .get_or_insert_with(|| meshes.add(hex_mesh(HEX_SIZE * 0.95)))
                        .clone();
                    parent.spawn((
                        HoverHighlightVisual,
                        Mesh2d(mesh_handle),
                        MeshMaterial2d(hex_materials.hover.clone()),
                        Transform::from_xyz(0.0, 0.0, HOVER_HIGHLIGHT_Z),
                    ));
                });
            }
        } else if let Some(existing_hover) = existing_hover {
            commands.entity(existing_hover).despawn();
        }

        let desired_dot = if attack_targets.contains(pos) {
            Some(TargetDotType::Attack)
        } else if move_targets.contains(pos) {
            Some(TargetDotType::Move)
        } else {
            None
        };

        let existing_dot = children.and_then(|children| {
            children
                .iter()
                .find(|child| dot_type_query.get(*child).is_ok())
        });

        if let Some(existing) = existing_dot {
            if desired_dot.is_none() {
                commands.entity(existing).despawn();
                continue;
            }
            if let Ok(current) = dot_type_query.get(existing) {
                if *current != desired_dot.unwrap() {
                    commands.entity(existing).despawn();
                } else {
                    continue;
                }
            }
        }

        if let Some(dot_type) = desired_dot {
            commands.entity(entity).with_children(|parent| {
                let mesh_handle = dot_mesh
                    .get_or_insert_with(|| meshes.add(RegularPolygon::new(HEX_SIZE * 0.18, 16)))
                    .clone();
                parent.spawn((
                    dot_type,
                    Mesh2d(mesh_handle),
                    MeshMaterial2d(hex_materials.target_dot.clone()),
                    Transform::from_xyz(0.0, 0.0, TARGET_DOT_Z),
                ));
            });
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
    mut controller: ResMut<Controller>,
    ui_state: Res<State<UiState>>,
    mut next_ui_state: ResMut<NextState<UiState>>,
    button_press_query: Query<&Interaction, (With<Button>, Changed<Interaction>)>,
    units: Query<(Entity, &Unit, &Owner, &HexPosition)>,
    cities: Query<(&HexPosition, &CityOwner), With<City>>,
    registry: Res<UnitRegistry>,
    terrain_tiles: Query<(&HexPosition, &Terrain), With<HexTile>>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    if button_press_query
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
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
    if local_player_game_over(&controller, &defeated, &victorious) {
        next_ui_state.set(UiState::Input {
            selection: InputSelection::Idle,
        });
        controller.selected_city = None;
        return;
    }

    // is the click on one of my owned units?
    let owned_unit_at = |hex: HexPosition| -> Option<Entity> {
        for (entity, _unit, owner, pos) in &units {
            if owner.0 == player_entity && *pos == hex {
                return Some(entity);
            }
        }
        None
    };

    if ui_state.is_locked() {
        return;
    }

    match ui_state.selection() {
        Some(InputSelection::Idle) => {
            if let Some(entity) = owned_unit_at(target) {
                controller.selected_city = None;
                next_ui_state.set(UiState::Input {
                    selection: InputSelection::UnitSelected { unit: entity },
                });
            }
        }
        Some(InputSelection::UnitSelected { .. }) => {
            if let Some(entity) = owned_unit_at(target) {
                controller.selected_city = None;
                next_ui_state.set(UiState::Input {
                    selection: InputSelection::UnitSelected { unit: entity },
                });
            } else {
                next_ui_state.set(UiState::Input {
                    selection: InputSelection::Idle,
                });
            }
        }
        Some(InputSelection::Targeting { unit, verb }) => {
            let Ok((_, u, _, pos)) = units.get(*unit) else {
                next_ui_state.set(UiState::Input {
                    selection: InputSelection::Idle,
                });
                return;
            };
            let Some(def) = registry.get(&u.type_id) else {
                next_ui_state.set(UiState::Input {
                    selection: InputSelection::Idle,
                });
                return;
            };
            match verb {
                TargetableVerb::Move => {
                    // A click in Move mode commits a move to `target`, including onto a
                    // tile a friendly currently occupies — that is how follow-moves,
                    // swaps, and rotations are issued: the occupant may vacate this turn
                    // (all moves resolve simultaneously), and the server rolls the move
                    // back if the tile isn't actually freed. Reselecting a different
                    // unit happens from the selected state, before entering Move mode.
                    let city_at_target = cities.iter().find(|(city_pos, _)| **city_pos == target);
                    let valid_city_target = city_at_target.is_none_or(|(_, city_owner)| {
                        city_owner.entity == player_entity || def.attack_range == 1
                    });
                    // Validate with the SAME shared reachability the preview used and
                    // the server will re-check, so an accepted click never bounces.
                    let terrain_map: std::collections::HashMap<HexPosition, Terrain> =
                        terrain_tiles.iter().map(|(p, t)| (*p, *t)).collect();
                    let reachable =
                        is_reachable(pos, &target, def, |p| terrain_map.get(p).copied());
                    if reachable && valid_city_target {
                        commands.client_trigger(UnitActionEvent {
                            unit: *unit,
                            action: UnitAction::Move { target },
                        });
                        next_ui_state.set(UiState::Input {
                            selection: InputSelection::Idle,
                        });
                    } else {
                        // invalid hex → fall back to selection state, bar stays
                        next_ui_state.set(UiState::Input {
                            selection: InputSelection::UnitSelected { unit: *unit },
                        });
                    }
                }
                TargetableVerb::Attack => {
                    // Clicking one of your own units isn't an attack target — treat it
                    // as switching the selection instead.
                    if let Some(entity) = owned_unit_at(target) {
                        controller.selected_city = None;
                        next_ui_state.set(UiState::Input {
                            selection: InputSelection::UnitSelected { unit: entity },
                        });
                        return;
                    }
                    // attacker is at `pos`; enemies are units with a different owner_id at `target`
                    let enemy_here = units
                        .iter()
                        .any(|(_, _, owner, p)| *p == target && owner.0 != player_entity)
                        || cities
                            .iter()
                            .any(|(p, owner)| *p == target && owner.entity != player_entity);
                    if is_within_attack_range(pos, &target, def.attack_range) && enemy_here {
                        commands.client_trigger(UnitActionEvent {
                            unit: *unit,
                            action: UnitAction::Attack { target },
                        });
                        next_ui_state.set(UiState::Input {
                            selection: InputSelection::Idle,
                        });
                    } else {
                        next_ui_state.set(UiState::Input {
                            selection: InputSelection::UnitSelected { unit: *unit },
                        });
                    }
                }
            }
        }
        None => {}
    }
}

/// Allows selecting both unit/city when they are on the same tile. This is a temporary solution
/// Better handling of user input / gui should be considered in the future
#[allow(clippy::too_many_arguments)]
pub fn handle_right_click(
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: CursorWorld,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    mut controller: ResMut<Controller>,
    mut next_ui_state: ResMut<NextState<UiState>>,
    cities: Query<(Entity, &HexPosition), With<City>>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
) {
    if !mouse.just_pressed(MouseButton::Right) {
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
    if local_player_game_over(&controller, &defeated, &victorious) {
        next_ui_state.set(UiState::Input {
            selection: InputSelection::Idle,
        });
        controller.selected_city = None;
        return;
    }

    let Some(target) = get_cursor_hex(&cursor) else {
        return;
    };

    // handle clicking city
    for (city_entity, pos) in cities {
        if *pos == target {
            next_ui_state.set(UiState::Input {
                selection: InputSelection::Idle,
            });
            controller.selected_city = Some(city_entity);
            println!("Selected city {city_entity}");
            return;
        }
    }
}

pub fn handle_escape_key(
    keys: Res<ButtonInput<KeyCode>>,
    ui_state: Res<State<UiState>>,
    mut next_ui_state: ResMut<NextState<UiState>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    let new_state = match ui_state.selection() {
        Some(InputSelection::Targeting { unit, .. }) => UiState::Input {
            selection: InputSelection::UnitSelected { unit: *unit },
        },
        _ => UiState::Input {
            selection: InputSelection::Idle,
        },
    };
    next_ui_state.set(new_state);
}

// drops UiState back to Idle if the unit it references no longer exists
pub fn prune_stale_selection(
    ui_state: Res<State<UiState>>,
    mut next_ui_state: ResMut<NextState<UiState>>,
    units: Query<(), With<Unit>>,
) {
    let referenced = match ui_state.selection() {
        Some(InputSelection::UnitSelected { unit }) => *unit,
        Some(InputSelection::Targeting { unit, .. }) => *unit,
        _ => return,
    };
    if units.get(referenced).is_err() {
        next_ui_state.set(UiState::Input {
            selection: InputSelection::Idle,
        });
    }
}

pub fn reset_submission_on_new_turn(
    turn_state: Query<&TurnState, Changed<TurnState>>,
    mut last_submitted: ResMut<LastSubmittedTurn>,
    mut last_logged_turn: Local<Option<u32>>,
) {
    for state in &turn_state {
        // Guard: only act once per distinct turn_number value.
        if *last_logged_turn == Some(state.turn_number) {
            continue;
        }
        *last_logged_turn = Some(state.turn_number);
        // When a new match starts turn_number resets to 0 while last_submitted
        // still holds the final turn of the previous match. Clear it so that
        // handle_left_click doesn't block input on the first turn of the new match.
        if last_submitted.0.is_some_and(|t| t > state.turn_number) {
            last_submitted.0 = None;
        }
        if let Some(submitted) = last_submitted.0
            && state.turn_number > submitted
        {
            println!("New turn {}! Ready to move.", state.turn_number);
        }
    }
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastSubmittedTurn>()
            .init_resource::<HoveredHex>()
            .init_resource::<Controller>()
            .add_systems(
                Update,
                (
                    handle_left_click,
                    handle_right_click,
                    handle_escape_key,
                    prune_stale_selection,
                    reset_submission_on_new_turn,
                ),
            );
    }
}
