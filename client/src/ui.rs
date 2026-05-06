use bevy::prelude::*;
use bevy_replicon::prelude::ClientTriggerExt;
use shared::components::*;
use shared::events::FinishTurn;

use crate::LocalPlayerColor;
use crate::input::{Controller, LastSubmittedTurn};

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
    mut controller: ResMut<Controller>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };

    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            if last_submitted.0 != Some(state.turn_number) {
                commands.client_trigger(FinishTurn);
                last_submitted.0 = Some(state.turn_number);
                controller.selected_unit = None;
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
                format!("Turn {} -- Click a neighbor hex to move", state.turn_number)
            }
        }
    };

    **text = message;
}
