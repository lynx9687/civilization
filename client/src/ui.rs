use bevy::prelude::*;
use shared::components::*;

use crate::LocalPlayerColor;
use crate::input::LastSubmittedTurn;

#[derive(Component)]
pub struct TurnUiText;

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
