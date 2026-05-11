use bevy::prelude::*;
use bevy_replicon::prelude::ClientTriggerExt;
use shared::cities::{City, CityOwner, CityStats};
use shared::components::*;
use shared::events::{CityAction, CityActionEvent, FinishTurn, UnitAction, UnitActionEvent};
use shared::production::{CityProduction, ProductionOutput, ProductionRecipeId, RecipeRegistry};
use shared::unit_definition::{UnitRegistry, UnitVerb, available_verbs};
use shared::units::Unit;

use crate::LocalPlayerColor;
use crate::input::{Controller, LastSubmittedTurn, TargetableVerb, UiState};

pub struct ColorState {
    pub idle: Color,
    pub hover: Color,
    pub pressed: Color,
    pub waiting: Color,
}

pub struct ButtonTheme {
    pub background: ColorState,
    pub border: ColorState,
}

pub mod theme {
    use super::*;
    use bevy::color::palettes::css::*;

    pub const FINISH_BUTTON: ButtonTheme = ButtonTheme {
        background: ColorState {
            idle: Color::Srgba(DARK_CYAN),
            hover: Color::Srgba(LIGHT_CYAN),
            pressed: Color::Srgba(DARK_SLATE_GRAY),
            waiting: Color::Srgba(DARK_GRAY),
        },
        border: ColorState {
            idle: Color::Srgba(TEAL),
            hover: Color::Srgba(AQUAMARINE),
            pressed: Color::Srgba(SLATE_GRAY),
            waiting: Color::Srgba(GRAY),
        },
    };
}

#[derive(Component)]
pub struct TurnUiText;

#[derive(Component)]
pub struct FinishTurnButton;

#[derive(Component)]
pub struct CityUiText;

#[derive(Component)]
pub struct ActionBar;

#[derive(Component)]
pub struct VerbButton(pub UnitVerb);

#[derive(Component)]
pub struct ProductionBar;

#[derive(Component)]
pub struct ProductionButton(pub Option<ProductionRecipeId>);

pub fn spawn_turn_ui(mut commands: Commands) {
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
    commands
        .spawn((
            FinishTurnButton,
            Button,
            Node {
                width: Val::Px(220.0),
                height: Val::Px(80.0),
                border: UiRect::all(Val::Px(4.0)),
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(Val::Px(10.0)),
                position_type: PositionType::Absolute,
                align_items: AlignItems::Center,
                right: Val::Px(20.0),
                bottom: Val::Px(20.0),
                ..Default::default()
            },
            BorderColor::from(theme::FINISH_BUTTON.border.idle),
            BackgroundColor::from(theme::FINISH_BUTTON.background.idle),
        ))
        .with_child((
            Text::new("FINISH TURN"),
            TextFont {
                font_size: 28.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));

    commands.spawn((
        CityUiText,
        Text::new("No city selected"),
        TextFont {
            font_size: 18.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(48.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
    // bottom-left action bar; hidden while UiState == Idle
    commands
        .spawn((
            ActionBar,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(10.0),
                left: Val::Px(10.0),
                display: Display::None,
                column_gap: Val::Px(6.0),
                ..default()
            },
        ))
        .with_children(|parent| {
            for verb in [
                UnitVerb::Move,
                UnitVerb::Attack,
                UnitVerb::Fortify,
                UnitVerb::Build,
                UnitVerb::Skip,
            ] {
                parent
                    .spawn((
                        VerbButton(verb),
                        Button,
                        Node {
                            width: Val::Px(80.0),
                            height: Val::Px(40.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BorderColor::all(Color::linear_rgb(1.0, 0.8, 0.2)),
                        BackgroundColor(Color::BLACK),
                    ))
                    .with_child(Text::new(verb_label(verb)));
            }
        });

    commands.spawn((
        ProductionBar,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(58.0),
            left: Val::Px(10.0),
            display: Display::None,
            column_gap: Val::Px(6.0),
            ..default()
        },
    ));
}

#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum PlayerTurnPhase {
    #[default]
    Input,
    Waiting,
}

pub fn finish_turn_trigger_system(
    mut commands: Commands,
    interaction_query: Query<&Interaction, (With<FinishTurnButton>, Changed<Interaction>)>,
    mut next_phase: ResMut<NextState<PlayerTurnPhase>>,
    turn_state: Query<&TurnState>,
    mut last_submitted: ResMut<LastSubmittedTurn>,
    mut ui_state: ResMut<UiState>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };

    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            if last_submitted.0 != Some(state.turn_number) {
                commands.client_trigger(FinishTurn);
                last_submitted.0 = Some(state.turn_number);
                *ui_state = UiState::Idle;
                next_phase.set(PlayerTurnPhase::Waiting);
            }
        }
    }
}

pub fn finish_turn_visual_system(
    mut button_query: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<FinishTurnButton>,
    >,
    phase: Res<State<PlayerTurnPhase>>,
) {
    let thm = &theme::FINISH_BUTTON;

    for (interaction, mut bg, mut border) in &mut button_query {
        if *phase.get() == PlayerTurnPhase::Waiting {
            *bg = thm.background.waiting.into();
            *border = thm.border.waiting.into();
            continue;
        }

        match *interaction {
            Interaction::Pressed => {
                *bg = thm.background.pressed.into();
                *border = thm.border.pressed.into();
            }
            Interaction::Hovered => {
                *bg = thm.background.hover.into();
                *border = thm.border.hover.into();
            }
            Interaction::None => {
                *bg = thm.background.idle.into();
                *border = thm.border.idle.into();
            }
        }
    }
}

pub fn reset_player_turn_phase(
    mut next_phase: ResMut<NextState<PlayerTurnPhase>>,
    turn_state: Query<&TurnState, Changed<TurnState>>,
) {
    if !turn_state.is_empty() {
        next_phase.set(PlayerTurnPhase::Input);
    }
}

pub fn update_turn_ui(
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
                format!(
                    "Turn {} -- Select a unit, then pick an action",
                    state.turn_number
                )
            }
        }
    };

    **text = message;
}

pub fn update_city_ui(
    controller: Res<Controller>,
    cities: Query<(&City, &CityOwner, &CityStats, Option<&CityProduction>)>,
    players: Query<&Player>,
    units: Res<UnitRegistry>,
    mut ui_text: Query<&mut Text, With<CityUiText>>,
) {
    let Ok(mut text) = ui_text.single_mut() else {
        return;
    };

    let Some(city_entity) = controller.selected_city else {
        **text = "No city selected".to_string();
        return;
    };

    let Ok((_city, owner, stats, production)) = cities.get(city_entity) else {
        **text = "No city selected".to_string();
        return;
    };

    let player_gold = players
        .get(owner.entity)
        .map_or(-99999, |player| player.gold);

    let production_text = production
        .and_then(|production| {
            production.recipe.map(|recipe| {
                format_recipe_progress(recipe.output, production.accumulated, recipe.cost, &units)
            })
        })
        .unwrap_or_else(|| "None".to_string());

    **text = format!(
        "City {}\nPopulation: {}\nFood: {} (+{})\nProduction: {}\nProducing: {}\nGold: +{} / owner {}",
        city_entity,
        stats.population,
        stats.food,
        stats.food_per_turn,
        stats.production,
        production_text,
        stats.gold_per_turn,
        player_gold
    );
}

pub fn populate_production_bar(
    mut commands: Commands,
    mut populated: Local<bool>,
    bars: Query<Entity, With<ProductionBar>>,
    recipes: Option<Res<RecipeRegistry>>,
    units: Option<Res<UnitRegistry>>,
) {
    if *populated {
        return;
    }
    let Some(recipes) = recipes else {
        return;
    };
    let Some(units) = units else {
        return;
    };
    let Ok(bar) = bars.single() else {
        return;
    };

    commands.entity(bar).with_children(|parent| {
        parent
            .spawn((
                ProductionButton(None),
                Button,
                production_button_node(),
                BorderColor::all(Color::linear_rgb(1.0, 0.8, 0.2)),
                BackgroundColor(Color::BLACK),
            ))
            .with_child(Text::new("None"));

        let mut recipe_list: Vec<_> = recipes.iter().collect();
        recipe_list.sort_by_key(|(id, _)| id.0);
        for (id, recipe) in recipe_list {
            parent
                .spawn((
                    ProductionButton(Some(*id)),
                    Button,
                    production_button_node(),
                    BorderColor::all(Color::linear_rgb(1.0, 0.8, 0.2)),
                    BackgroundColor(Color::BLACK),
                ))
                .with_child(Text::new(recipe_button_label(recipe.output, &units)));
        }
    });

    *populated = true;
}

fn production_button_node() -> Node {
    Node {
        width: Val::Px(92.0),
        height: Val::Px(36.0),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        border: UiRect::all(Val::Px(2.0)),
        ..default()
    }
}

fn recipe_button_label(output: ProductionOutput, units: &UnitRegistry) -> String {
    match output {
        ProductionOutput::Unit { type_id } => units.name_of(type_id).unwrap_or("Unit").to_string(),
    }
}

fn format_recipe_progress(
    output: ProductionOutput,
    accumulated: u32,
    cost: u32,
    units: &UnitRegistry,
) -> String {
    format!(
        "{} {}/{}",
        recipe_button_label(output, units),
        accumulated,
        cost
    )
}

pub fn update_production_bar(
    controller: Res<Controller>,
    cities: Query<&CityOwner, With<City>>,
    mut bars: Query<&mut Node, With<ProductionBar>>,
) {
    if !controller.is_changed() {
        return;
    }

    let show = controller
        .selected_city
        .and_then(|city| cities.get(city).ok())
        .is_some_and(|owner| Some(owner.entity) == controller.player_entity);

    for mut node in &mut bars {
        node.display = if show { Display::Flex } else { Display::None };
    }
}

// reacts to UiState changes: shows/hides bar, sets enabled/greyed buttons
pub fn update_action_bar(
    ui_state: Res<UiState>,
    mut bars: Query<&mut Node, (With<ActionBar>, Without<VerbButton>)>,
    mut buttons: Query<(&VerbButton, &mut BackgroundColor)>,
    units: Query<&Unit>,
    registry: Res<UnitRegistry>,
) {
    if !ui_state.is_changed() {
        return;
    }
    let unit_entity = match *ui_state {
        UiState::Idle => {
            for mut node in &mut bars {
                node.display = Display::None;
            }
            return;
        }
        UiState::UnitSelected { unit } => unit,
        UiState::Targeting { unit, .. } => unit,
    };

    for mut node in &mut bars {
        node.display = Display::Flex;
    }

    let Ok(unit) = units.get(unit_entity) else {
        return;
    };
    let Some(def) = registry.get(&unit.type_id) else {
        return;
    };
    let available = available_verbs(def);

    for (button, mut bg) in &mut buttons {
        if available.contains(&button.0) {
            *bg = BackgroundColor(Color::BLACK);
        } else {
            // greyed: visually distinct but click handler will also reject
            *bg = BackgroundColor(Color::srgb(0.2, 0.2, 0.2));
        }
    }
}

fn verb_label(v: UnitVerb) -> &'static str {
    match v {
        UnitVerb::Move => "Move",
        UnitVerb::Attack => "Attack",
        UnitVerb::Fortify => "Fortify",
        UnitVerb::Build => "Build",
        UnitVerb::Skip => "Skip",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_verb_button_click(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<&VerbButton>,
    mut ui_state: ResMut<UiState>,
    units: Query<&Unit>,
    registry: Res<UnitRegistry>,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
) {
    let Ok(VerbButton(verb)) = buttons.get(click.entity) else {
        return;
    };

    // gate: only act during Accepting phase and before the player has finished
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }
    if last_submitted.0.is_some_and(|t| t >= state.turn_number) {
        return;
    }

    let unit_entity = match *ui_state {
        UiState::UnitSelected { unit } => unit,
        UiState::Targeting { unit, .. } => unit,
        UiState::Idle => return,
    };

    let Ok(unit) = units.get(unit_entity) else {
        return;
    };
    let Some(def) = registry.get(&unit.type_id) else {
        return;
    };
    if !available_verbs(def).contains(verb) {
        return;
    }

    match verb {
        UnitVerb::Move => {
            *ui_state = match *ui_state {
                // re-clicking the targeting verb toggles back to UnitSelected
                UiState::Targeting {
                    unit,
                    verb: TargetableVerb::Move,
                } => UiState::UnitSelected { unit },
                _ => UiState::Targeting {
                    unit: unit_entity,
                    verb: TargetableVerb::Move,
                },
            };
        }
        UnitVerb::Attack => {
            *ui_state = match *ui_state {
                UiState::Targeting {
                    unit,
                    verb: TargetableVerb::Attack,
                } => UiState::UnitSelected { unit },
                _ => UiState::Targeting {
                    unit: unit_entity,
                    verb: TargetableVerb::Attack,
                },
            };
        }
        UnitVerb::Fortify => {
            commands.client_trigger(UnitActionEvent {
                unit: unit_entity,
                action: UnitAction::Fortify,
            });
            *ui_state = UiState::Idle;
        }
        UnitVerb::Skip => {
            commands.client_trigger(UnitActionEvent {
                unit: unit_entity,
                action: UnitAction::Skip,
            });
            *ui_state = UiState::Idle;
        }
        UnitVerb::Build => {
            // single-target stub: only one project per unit today (settler→city);
            // multi-target Build needs a sub-menu — see future-extensions in spec
            let Some(project) = def.build_targets.first() else {
                return;
            };
            commands.client_trigger(UnitActionEvent {
                unit: unit_entity,
                action: UnitAction::Build {
                    project: project.clone(),
                },
            });
            *ui_state = UiState::Idle;
        }
    }
}

pub fn handle_production_button_click(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<&ProductionButton>,
    controller: Res<Controller>,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
) {
    let Ok(button) = buttons.get(click.entity) else {
        return;
    };
    let Some(city) = controller.selected_city else {
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

    let action = match button.0 {
        Some(recipe_id) => CityAction::SetProduction { recipe_id },
        None => CityAction::ClearProduction,
    };
    commands.client_trigger(CityActionEvent { city, action });
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(handle_verb_button_click)
            .add_observer(handle_production_button_click)
            .add_systems(
                Update,
                (
                    populate_production_bar,
                    update_turn_ui,
                    update_city_ui,
                    update_action_bar,
                    update_production_bar,
                    finish_turn_trigger_system.run_if(in_state(PlayerTurnPhase::Input)),
                    finish_turn_visual_system,
                    reset_player_turn_phase,
                ),
            );
    }
}
