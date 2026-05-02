use bevy::prelude::*;
use bevy_replicon::prelude::ClientTriggerExt;
use shared::cities::{City, CityStats};
use shared::components::*;
use shared::events::FinishTurn;
use shared::units::Owner;

use crate::LocalPlayerColor;
use crate::input::{Controller, LastSubmittedTurn};

#[derive(Component)]
pub struct TurnUiText;

#[derive(Component)]
pub struct FinishTurnButton;

#[derive(Component)]
pub struct CityUiText;

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
}

pub fn finish_turn_clicked(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<(), With<FinishTurnButton>>,
    turn_state: Query<&TurnState>,
    mut last_submitted: ResMut<LastSubmittedTurn>,
    mut controller: ResMut<Controller>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if buttons.contains(click.entity) {
        commands.client_trigger(FinishTurn);
        last_submitted.0 = Some(state.turn_number);
        controller.selected_unit = None;
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

pub fn update_city_ui(
    controller: Res<Controller>,
    cities: Query<(&City, &Owner, &CityStats)>,
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

    let Ok((city, owner, stats)) = cities.get(city_entity) else {
        **text = "No city selected".to_string();
        return;
    };

    let player_gold = players
        .iter()
        .find(|player| player.player_id == owner.player_id)
        .map_or(0, |player| player.gold);

    **text = format!(
        "City {}\nPopulation: {}\nFood: {} (+{})\nProduction: {}\nGold: +{} / owner {}",
        city.id,
        stats.population,
        stats.food,
        stats.food_per_turn,
        stats.production,
        stats.gold_per_turn,
        player_gold
    );
}
