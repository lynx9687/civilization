use bevy::prelude::*;
use bevy_replicon::prelude::ClientTriggerExt;
use shared::cities::{City, CityOwner, CityStats};
use shared::components::*;
use shared::events::{FinishTurn, UnitAction, UnitActionEvent};
use shared::unit_definition::{UnitRegistry, UnitVerb, available_verbs};
use shared::units::Unit;

use crate::LocalPlayerColor;
use crate::input::{Controller, LastSubmittedTurn, TargetableVerb, UiState};

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
                width: Val::Px(150.0),
                height: Val::Px(100.0),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::MAX,
                position_type: PositionType::Absolute,
                align_items: AlignItems::Center,
                right: Val::Px(10.0),
                bottom: Val::Px(10.0),
                ..Default::default()
            },
            BorderColor::all(Color::linear_rgb(1.0, 0.8, 0.2)),
            BackgroundColor(Color::BLACK),
        ))
        .with_child((Text::new("Finish Turn"),));

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
}

pub fn finish_turn_clicked(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<(), With<FinishTurnButton>>,
    turn_state: Query<&TurnState>,
    mut last_submitted: ResMut<LastSubmittedTurn>,
    mut ui_state: ResMut<UiState>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if buttons.contains(click.entity) {
        commands.client_trigger(FinishTurn);
        last_submitted.0 = Some(state.turn_number);
        *ui_state = UiState::Idle;
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
    cities: Query<(&City, &CityOwner, &CityStats)>,
    players: Query<&Player>,
    mut ui_text: Query<&mut Text, With<CityUiText>>,
) {
    let Ok(mut text) = ui_text.single_mut() else {
        return;
    };

    let Some(city_entity) = controller.selected_city else {
        **text = "No city selected".to_string();
        return;
    };

    let Ok((_city, owner, stats)) = cities.get(city_entity) else {
        **text = "No city selected".to_string();
        return;
    };

    let player_gold = players
        .get(owner.entity)
        .map_or(-99999, |player| player.gold);

    **text = format!(
        "City {}\nPopulation: {}\nFood: {} (+{})\nProduction: {}\nGold: +{} / owner {}",
        city_entity,
        stats.population,
        stats.food,
        stats.food_per_turn,
        stats.production,
        stats.gold_per_turn,
        player_gold
    );
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
