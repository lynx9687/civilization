use bevy::prelude::*;
use bevy_replicon::prelude::ClientTriggerExt;
use shared::cities::{City, CityOwner, CityStats};
use shared::components::*;
use shared::events::{CityAction, CityActionEvent, FinishTurn, UnitAction, UnitActionEvent};
use shared::hex::HexPosition;
use shared::production::{CityProduction, ProductionOutput, ProductionRecipeId, RecipeRegistry};
use shared::unit_definition::{UnitRegistry, UnitVerb, available_verbs};
use shared::units::{ColorIndex, Health, Owner, Unit};

use crate::input::{
    Controller, HoveredHex, InputSelection, LastSubmittedTurn, TargetableVerb, UiState,
    local_player_defeated, local_player_game_over, local_player_victorious,
};
use crate::visuals::theme;

const TOOLTIP_WIDTH: f32 = 230.0;
const TOOLTIP_HEIGHT: f32 = 154.0;
const TOOLTIP_CURSOR_OFFSET: f32 = 18.0;

#[derive(Component)]
pub struct TurnUiText;

#[derive(Component)]
pub struct FinishTurnButton;

#[derive(Component)]
pub struct CityUiText;

#[derive(Component)]
pub struct ActionBar;

#[derive(Component)]
pub struct ActionButton;

#[derive(Component)]
pub struct VerbButton(pub UnitVerb);

#[derive(Component)]
pub struct ProductionBar;

#[derive(Component)]
pub struct ProductionButton(pub Option<ProductionRecipeId>);

#[derive(Component)]
pub struct LoseScreen;

#[derive(Component)]
pub struct VictoryScreen;

#[derive(Component)]
pub struct PlayerColorIndicator;

#[derive(Component)]
pub struct PlayerColorSwatch;

#[derive(Component)]
pub struct PlayerColorText;

#[derive(Component)]
pub struct UnitTooltip;

#[derive(Component)]
pub struct UnitTooltipTitle;

#[derive(Component)]
pub struct UnitTooltipSubtitle;

#[derive(Component)]
pub struct UnitTooltipStats;

type UnitTooltipTitleFilter = (
    With<UnitTooltipTitle>,
    Without<UnitTooltipSubtitle>,
    Without<UnitTooltipStats>,
);
type UnitTooltipSubtitleFilter = (
    With<UnitTooltipSubtitle>,
    Without<UnitTooltipTitle>,
    Without<UnitTooltipStats>,
);
type UnitTooltipStatsFilter = (
    With<UnitTooltipStats>,
    Without<UnitTooltipTitle>,
    Without<UnitTooltipSubtitle>,
);

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

    commands
        .spawn((
            PlayerColorIndicator,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(10.0),
                right: Val::Px(20.0),
                height: Val::Px(42.0),
                display: Display::None,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.2)),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.72)),
        ))
        .with_children(|parent| {
            parent.spawn((
                PlayerColorSwatch,
                Node {
                    width: Val::Px(20.0),
                    height: Val::Px(20.0),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                },
                BorderColor::all(Color::WHITE),
                BackgroundColor(Color::WHITE),
            ));
            parent.spawn((
                PlayerColorText,
                Text::new("Your color"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });

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
                        ActionButton,
                        Button,
                        Node {
                            width: Val::Px(80.0),
                            height: Val::Px(40.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BorderColor::from(theme::FINISH_BUTTON.border.idle),
                        BackgroundColor::from(theme::FINISH_BUTTON.background.idle),
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

    commands.spawn((
        LoseScreen,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            display: Display::None,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(16.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
        GlobalZIndex(100),
        children![
            (
                Text::new("You lost"),
                TextFont {
                    font_size: 64.0,
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.1, 0.1)),
            ),
            (
                Text::new("Close the app to leave the game."),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            )
        ],
    ));

    commands.spawn((
        VictoryScreen,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            display: Display::None,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(16.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
        GlobalZIndex(100),
        children![
            (
                Text::new("You won"),
                TextFont {
                    font_size: 64.0,
                    ..default()
                },
                TextColor(Color::srgb(0.1, 0.85, 0.25)),
            ),
            (
                Text::new("Close the app to leave the game."),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            )
        ],
    ));

    commands
        .spawn((
            UnitTooltip,
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                width: Val::Px(TOOLTIP_WIDTH),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(7.0),
                ..default()
            },
            BorderColor::all(Color::srgba(0.75, 0.88, 1.0, 0.55)),
            BackgroundColor(Color::srgba(0.02, 0.03, 0.045, 0.92)),
            GlobalZIndex(50),
        ))
        .with_children(|parent| {
            parent.spawn((
                Node {
                    width: Val::Px(40.0),
                    height: Val::Px(3.0),
                    border_radius: BorderRadius::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.95, 0.74, 0.32)),
            ));
            parent.spawn((
                UnitTooltipTitle,
                Text::new("Archer"),
                TextFont {
                    font_size: 22.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            parent.spawn((
                UnitTooltipSubtitle,
                Text::new("Ranged unit"),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgba(0.78, 0.86, 0.95, 0.88)),
            ));
            parent.spawn((
                UnitTooltipStats,
                Text::new("HP 8/8    Move 2\nAttack 3  Range 2\nCost 25"),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::srgba(0.94, 0.96, 0.98, 1.0)),
            ));
        });
}

#[allow(clippy::too_many_arguments)]
pub fn finish_turn_trigger_system(
    mut commands: Commands,
    interaction_query: Query<&Interaction, (With<FinishTurnButton>, Changed<Interaction>)>,
    mut next_ui_state: ResMut<NextState<UiState>>,
    turn_state: Query<&TurnState>,
    mut last_submitted: ResMut<LastSubmittedTurn>,
    controller: Res<Controller>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
) {
    if local_player_game_over(&controller, &defeated, &victorious) {
        return;
    }
    let Ok(state) = turn_state.single() else {
        return;
    };

    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed && last_submitted.0 != Some(state.turn_number) {
            commands.client_trigger(FinishTurn);
            last_submitted.0 = Some(state.turn_number);
            next_ui_state.set(UiState::Locked);
        }
    }
}

pub fn finish_turn_visual_system(
    mut button_query: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<FinishTurnButton>,
    >,
    ui_state: Res<State<UiState>>,
) {
    let thm = &theme::FINISH_BUTTON;

    for (interaction, mut bg, mut border) in &mut button_query {
        if ui_state.is_locked() {
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

pub fn reset_ui_state_on_turn_state_change(
    mut next_ui_state: ResMut<NextState<UiState>>,
    turn_state: Query<&TurnState, Changed<TurnState>>,
) {
    if !turn_state.is_empty() {
        next_ui_state.set(UiState::Input {
            selection: InputSelection::Idle,
        });
    }
}

pub fn update_turn_ui(
    turn_state: Query<&TurnState>,
    controller: Res<Controller>,
    last_submitted: Res<LastSubmittedTurn>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
    mut ui_text: Query<&mut Text, With<TurnUiText>>,
) {
    let Ok(mut text) = ui_text.single_mut() else {
        return;
    };

    if controller.player_entity.is_none() {
        **text = "Connecting...".to_string();
        return;
    }
    if local_player_defeated(&controller, &defeated) {
        **text = "You lost".to_string();
        return;
    }
    if local_player_victorious(&controller, &victorious) {
        **text = "You won".to_string();
        return;
    }

    let Ok(state) = turn_state.single() else {
        **text = "Waiting for game state...".to_string();
        return;
    };

    let message = match state.phase {
        TurnPhase::Lobby => "In lobby — waiting for game to start...".to_string(),
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

pub fn update_player_color_indicator(
    controller: Res<Controller>,
    turn_state: Query<&TurnState>,
    players: Query<&Player>,
    mut indicators: Query<&mut Node, With<PlayerColorIndicator>>,
    mut swatches: Query<(&mut BackgroundColor, &mut BorderColor), With<PlayerColorSwatch>>,
    mut labels: Query<&mut Text, With<PlayerColorText>>,
) {
    let show_ingame = turn_state
        .single()
        .is_ok_and(|state| state.phase != TurnPhase::Lobby);
    let Some(player_entity) = controller.player_entity else {
        for mut node in &mut indicators {
            node.display = Display::None;
        }
        return;
    };
    let Ok(player) = players.get(player_entity) else {
        for mut node in &mut indicators {
            node.display = Display::None;
        }
        return;
    };

    for mut node in &mut indicators {
        node.display = if show_ingame {
            Display::Flex
        } else {
            Display::None
        };
    }

    let color = player_color(player.color_index);
    for (mut background, mut border) in &mut swatches {
        *background = BackgroundColor(color);
        *border = BorderColor::all(Color::WHITE);
    }

    for mut text in &mut labels {
        **text = format!("Player {}", player.color_index + 1);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_unit_tooltip(
    hovered_hex: Res<HoveredHex>,
    windows: Query<&Window>,
    units: Query<(&Unit, &Health, &Owner, &ColorIndex, &HexPosition)>,
    players: Query<&Player>,
    registry: Res<UnitRegistry>,
    mut tooltips: Query<&mut Node, With<UnitTooltip>>,
    mut titles: Query<&mut Text, UnitTooltipTitleFilter>,
    mut subtitles: Query<&mut Text, UnitTooltipSubtitleFilter>,
    mut stats: Query<&mut Text, UnitTooltipStatsFilter>,
) {
    let Ok(mut tooltip) = tooltips.single_mut() else {
        return;
    };

    let Some(hex) = hovered_hex.current() else {
        tooltip.display = Display::None;
        return;
    };
    let Some((unit, health, owner, color_index, _pos)) =
        units.iter().find(|(_, _, _, _, pos)| **pos == hex)
    else {
        tooltip.display = Display::None;
        return;
    };
    let Some(definition) = registry.get(&unit.type_id) else {
        tooltip.display = Display::None;
        return;
    };
    let Some(unit_name) = registry.name_of(unit.type_id) else {
        tooltip.display = Display::None;
        return;
    };
    let Ok(window) = windows.single() else {
        tooltip.display = Display::None;
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        tooltip.display = Display::None;
        return;
    };

    tooltip.display = Display::Flex;
    tooltip.left = Val::Px(
        (cursor.x + TOOLTIP_CURSOR_OFFSET)
            .min(window.width() - TOOLTIP_WIDTH)
            .max(0.0),
    );
    tooltip.top = Val::Px(
        (cursor.y + TOOLTIP_CURSOR_OFFSET)
            .min(window.height() - TOOLTIP_HEIGHT)
            .max(0.0),
    );

    let owner_label = players
        .get(owner.0)
        .map(|player| format!("Player {}", player.color_index + 1))
        .unwrap_or_else(|_| format!("Player {}", color_index.0 + 1));

    if let Ok(mut title) = titles.single_mut() {
        **title = title_case(unit_name);
    }
    if let Ok(mut subtitle) = subtitles.single_mut() {
        **subtitle = format!("{owner_label} unit");
    }
    if let Ok(mut stats) = stats.single_mut() {
        **stats = format!(
            "HP {}/{}    Move {}\nAttack {}  Range {}\nCost {}",
            health.current,
            health.max,
            definition.move_budget,
            definition.attack_damage,
            definition.attack_range,
            // definition.gold_upkeep,
            definition.production_cost
        );
    }
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
                ActionButton,
                Button,
                production_button_node(),
                BorderColor::from(theme::FINISH_BUTTON.border.idle),
                BackgroundColor::from(theme::FINISH_BUTTON.background.idle),
            ))
            .with_child(Text::new("None"));

        let mut recipe_list: Vec<_> = recipes.iter().collect();
        recipe_list.sort_by_key(|(id, _)| id.0);
        for (id, recipe) in recipe_list {
            parent
                .spawn((
                    ProductionButton(Some(*id)),
                    ActionButton,
                    Button,
                    production_button_node(),
                    BorderColor::from(theme::FINISH_BUTTON.border.idle),
                    BackgroundColor::from(theme::FINISH_BUTTON.background.idle),
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
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
    mut bars: Query<&mut Node, With<ProductionBar>>,
) {
    if !controller.is_changed() && defeated.is_empty() && victorious.is_empty() {
        return;
    }

    let show = !local_player_game_over(&controller, &defeated, &victorious)
        && controller
            .selected_city
            .and_then(|city| cities.get(city).ok())
            .is_some_and(|owner| Some(owner.entity) == controller.player_entity);

    for mut node in &mut bars {
        node.display = if show { Display::Flex } else { Display::None };
    }
}

// reacts to UiState changes: shows/hides bar, sets enabled/greyed buttons
#[allow(clippy::too_many_arguments)]
pub fn update_action_bar(
    ui_state: Res<State<UiState>>,
    mut bars: Query<&mut Node, (With<ActionBar>, Without<VerbButton>)>,
    mut buttons: Query<(&VerbButton, &mut BackgroundColor, &mut BorderColor)>,
    units: Query<&Unit>,
    registry: Res<UnitRegistry>,
    controller: Res<Controller>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
) {
    if !ui_state.is_changed() && defeated.is_empty() && victorious.is_empty() {
        return;
    }
    if local_player_game_over(&controller, &defeated, &victorious) {
        for mut node in &mut bars {
            node.display = Display::None;
        }
        return;
    }
    let unit_entity = match ui_state.selection() {
        Some(InputSelection::Idle) | None => {
            for mut node in &mut bars {
                node.display = Display::None;
            }
            return;
        }
        Some(InputSelection::UnitSelected { unit }) => *unit,
        Some(InputSelection::Targeting { unit, .. }) => *unit,
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

    for (button, mut bg, mut border) in &mut buttons {
        if available.contains(&button.0) {
            *bg = BackgroundColor::from(theme::FINISH_BUTTON.background.idle);
            *border = BorderColor::from(theme::FINISH_BUTTON.border.idle);
        } else {
            // greyed: visually distinct but click handler will also reject
            *bg = BackgroundColor(Color::srgb(0.2, 0.2, 0.2));
            *border = BorderColor::from(Color::srgba(0.5, 0.5, 0.5, 1.0));
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn action_button_visual_system(
    ui_state: Res<State<UiState>>,
    units: Query<&Unit>,
    registry: Res<UnitRegistry>,
    mut buttons: Query<
        (
            Option<&VerbButton>,
            Option<&ProductionButton>,
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        (With<ActionButton>, Changed<Interaction>),
    >,
) {
    let thm = &theme::FINISH_BUTTON;

    let is_disabled_verb = |verb_button: Option<&VerbButton>| -> bool {
        if let Some(VerbButton(verb)) = verb_button {
            let unit_entity = match ui_state.selection() {
                Some(InputSelection::UnitSelected { unit }) => *unit,
                Some(InputSelection::Targeting { unit, .. }) => *unit,
                _ => return false,
            };

            let Ok(unit) = units.get(unit_entity) else {
                return false;
            };
            let Some(def) = registry.get(&unit.type_id) else {
                return false;
            };
            !available_verbs(def).contains(verb)
        } else {
            false
        }
    };

    for (verb_button, _production_button, interaction, mut bg, mut border) in &mut buttons {
        if is_disabled_verb(verb_button) {
            *bg = BackgroundColor(Color::srgb(0.2, 0.2, 0.2));
            *border = BorderColor::from(Color::srgba(0.5, 0.5, 0.5, 1.0));
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

fn verb_label(v: UnitVerb) -> &'static str {
    match v {
        UnitVerb::Move => "Move",
        UnitVerb::Attack => "Attack",
        UnitVerb::Fortify => "Fortify",
        UnitVerb::Build => "Build",
        UnitVerb::Skip => "Skip",
    }
}

fn title_case(name: &str) -> String {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return "Unit".to_string();
    };

    first.to_uppercase().chain(chars).collect()
}

#[allow(clippy::too_many_arguments)]
pub fn handle_verb_button_click(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<&VerbButton>,
    ui_state: Res<State<UiState>>,
    mut next_ui_state: ResMut<NextState<UiState>>,
    units: Query<&Unit>,
    registry: Res<UnitRegistry>,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    controller: Res<Controller>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
) {
    if local_player_game_over(&controller, &defeated, &victorious) {
        next_ui_state.set(UiState::Input {
            selection: InputSelection::Idle,
        });
        return;
    }
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

    let unit_entity = match ui_state.selection() {
        Some(InputSelection::UnitSelected { unit }) => *unit,
        Some(InputSelection::Targeting { unit, .. }) => *unit,
        _ => return,
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
            next_ui_state.set(match ui_state.selection() {
                Some(InputSelection::Targeting {
                    unit,
                    verb: TargetableVerb::Move,
                }) => UiState::Input {
                    selection: InputSelection::UnitSelected { unit: *unit },
                },
                _ => UiState::Input {
                    selection: InputSelection::Targeting {
                        unit: unit_entity,
                        verb: TargetableVerb::Move,
                    },
                },
            });
        }
        UnitVerb::Attack => {
            next_ui_state.set(match ui_state.selection() {
                Some(InputSelection::Targeting {
                    unit,
                    verb: TargetableVerb::Attack,
                }) => UiState::Input {
                    selection: InputSelection::UnitSelected { unit: *unit },
                },
                _ => UiState::Input {
                    selection: InputSelection::Targeting {
                        unit: unit_entity,
                        verb: TargetableVerb::Attack,
                    },
                },
            });
        }
        UnitVerb::Fortify => {
            commands.client_trigger(UnitActionEvent {
                unit: unit_entity,
                action: UnitAction::Fortify,
            });
            next_ui_state.set(UiState::Input {
                selection: InputSelection::Idle,
            });
        }
        UnitVerb::Skip => {
            commands.client_trigger(UnitActionEvent {
                unit: unit_entity,
                action: UnitAction::Skip,
            });
            next_ui_state.set(UiState::Input {
                selection: InputSelection::Idle,
            });
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
            next_ui_state.set(UiState::Input {
                selection: InputSelection::Idle,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_production_button_click(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<&ProductionButton>,
    controller: Res<Controller>,
    turn_state: Query<&TurnState>,
    last_submitted: Res<LastSubmittedTurn>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
) {
    if local_player_game_over(&controller, &defeated, &victorious) {
        return;
    }
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

pub fn update_lose_screen(
    controller: Res<Controller>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
    mut screens: Query<&mut Node, With<LoseScreen>>,
    mut victory_screens: Query<&mut Node, (With<VictoryScreen>, Without<LoseScreen>)>,
    mut next_ui_state: ResMut<NextState<UiState>>,
) {
    let lost = local_player_defeated(&controller, &defeated);
    let won = local_player_victorious(&controller, &victorious);
    if lost || won {
        next_ui_state.set(UiState::Input {
            selection: InputSelection::Idle,
        });
    }
    for mut node in &mut screens {
        node.display = if lost { Display::Flex } else { Display::None };
    }
    for mut node in &mut victory_screens {
        node.display = if won { Display::Flex } else { Display::None };
    }
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(handle_verb_button_click)
            .add_observer(handle_production_button_click)
            .add_systems(Startup, spawn_turn_ui)
            .add_systems(
                Update,
                (
                    populate_production_bar,
                    update_turn_ui,
                    update_player_color_indicator,
                    update_unit_tooltip,
                    update_city_ui,
                    update_action_bar,
                    update_production_bar,
                    action_button_visual_system,
                    finish_turn_trigger_system,
                    finish_turn_visual_system,
                    reset_ui_state_on_turn_state_change,
                    update_lose_screen,
                ),
            );
    }
}
