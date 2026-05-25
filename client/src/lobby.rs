use bevy::prelude::*;
use bevy_replicon::prelude::ClientTriggerExt;
use shared::components::{
    DefeatedPlayer, Host, PLAYER_COLORS, Player, TurnPhase, TurnState, VictoriousPlayer,
    WaitingPlayer,
};
use shared::events::StartGame;

use crate::input::Controller;
use crate::input::local_player_game_over;

#[derive(Component)]
pub struct LobbyOverlay;

#[derive(Component)]
pub struct StartGameButton;

#[derive(Component)]
pub struct LobbyStatusText;

#[derive(Component)]
pub struct LobbyPlayerList;

/// Marker for lobby player rows; despawn all on rebuild.
#[derive(Component)]
pub struct LobbyPlayerRow;

pub fn spawn_lobby_ui(mut commands: Commands) {
    commands
        .spawn((
            LobbyOverlay,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.78)),
            GlobalZIndex(10),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(400.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(24.0)),
                    row_gap: Val::Px(14.0),
                    border: UiRect::all(Val::Px(3.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.05, 0.05, 0.05)),
                BorderColor::all(Color::linear_rgb(1.0, 0.8, 0.2)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("Lobby"),
                    TextFont {
                        font_size: 28.0,
                        ..default()
                    },
                    TextColor(Color::linear_rgb(1.0, 0.8, 0.2)),
                ));

                panel.spawn((
                    LobbyPlayerList,
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        width: Val::Percent(100.0),
                        row_gap: Val::Px(8.0),
                        min_height: Val::Px(40.0),
                        ..default()
                    },
                ));

                panel.spawn((
                    LobbyStatusText,
                    Text::new("Waiting for host to start the game..."),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.7, 0.7, 0.7)),
                    Node {
                        display: Display::None,
                        ..default()
                    },
                ));

                panel
                    .spawn((
                        StartGameButton,
                        Button,
                        Node {
                            width: Val::Px(180.0),
                            height: Val::Px(50.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(3.0)),
                            border_radius: BorderRadius::MAX,
                            display: Display::None,
                            ..default()
                        },
                        BorderColor::all(Color::linear_rgb(1.0, 0.8, 0.2)),
                        BackgroundColor(Color::BLACK),
                    ))
                    .with_child(Text::new("Start Game"));
            });
        });
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn update_lobby_ui(
    mut commands: Commands,
    turn_state: Query<&TurnState>,
    players: Query<(Entity, &Player)>,
    hosts: Query<Entity, With<Host>>,
    controller: Res<Controller>,
    waiting_players: Query<(), With<WaitingPlayer>>,
    mut overlay_nodes: Query<&mut Node, With<LobbyOverlay>>,
    mut start_btn_nodes: Query<
        &mut Node,
        (
            With<StartGameButton>,
            Without<LobbyOverlay>,
            Without<LobbyStatusText>,
            Without<LobbyPlayerList>,
        ),
    >,
    mut status_nodes: Query<
        &mut Node,
        (
            With<LobbyStatusText>,
            Without<LobbyOverlay>,
            Without<StartGameButton>,
            Without<LobbyPlayerList>,
        ),
    >,
    mut status_text: Query<&mut Text, With<LobbyStatusText>>,
    list_query: Query<Entity, With<LobbyPlayerList>>,
    existing_rows: Query<Entity, With<LobbyPlayerRow>>,
    mut last_players: Local<Vec<(u8, Entity)>>,
    mut last_host: Local<Option<Entity>>,
    mut last_me: Local<Option<Entity>>,
) {
    let in_lobby = turn_state
        .single()
        .map(|s| s.phase == TurnPhase::Lobby)
        .unwrap_or(true); // stay visible while TurnState hasn't arrived yet

    // A late-joining client has WaitingPlayer on their entity; they must see the
    // overlay even while the game is in progress for everyone else.
    let is_waiting = controller
        .player_entity
        .is_some_and(|e| waiting_players.contains(e));

    for mut node in &mut overlay_nodes {
        node.display = if in_lobby || is_waiting {
            Display::Flex
        } else {
            Display::None
        };
    }
    if !in_lobby && !is_waiting {
        return;
    }

    // Waiting-room path: show "game in progress" message, hide everything else.
    if is_waiting {
        for mut node in &mut start_btn_nodes {
            node.display = Display::None;
        }
        for mut node in &mut status_nodes {
            node.display = Display::Flex;
        }
        if let Ok(mut text) = status_text.single_mut() {
            **text = "A game is in progress. You will join the next one.".to_string();
        }
        for row_entity in &existing_rows {
            commands.entity(row_entity).despawn();
        }
        *last_players = vec![];
        *last_host = None;
        return;
    }

    // Normal lobby path below.
    if let Ok(mut text) = status_text.single_mut() {
        **text = "Waiting for host to start the game...".to_string();
    }

    let host_entity = hosts.single().ok();
    let i_am_host = host_entity
        .zip(controller.player_entity)
        .is_some_and(|(host, mine)| host == mine);
    let can_start = i_am_host && players.iter().count() >= 2;

    for mut node in &mut start_btn_nodes {
        node.display = if can_start {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut node in &mut status_nodes {
        node.display = if !i_am_host {
            Display::Flex
        } else {
            Display::None
        };
    }

    // Rebuild player list only when its contents change.
    // Sort by color_index so the display order matches the server's compact ordering.
    let mut sorted_players: Vec<(u8, Entity)> =
        players.iter().map(|(e, p)| (p.color_index, e)).collect();
    sorted_players.sort();

    let my_entity = controller.player_entity;
    let needs_rebuild =
        sorted_players != *last_players || host_entity != *last_host || my_entity != *last_me;
    if !needs_rebuild {
        return;
    }
    *last_players = sorted_players.clone();
    *last_host = host_entity;
    *last_me = my_entity;

    let Ok(list_entity) = list_query.single() else {
        return;
    };

    for row_entity in &existing_rows {
        commands.entity(row_entity).despawn();
    }

    let player_map: std::collections::HashMap<Entity, &Player> = players.iter().collect();

    for (slot, player_entity) in &sorted_players {
        let Some(player) = player_map.get(player_entity) else {
            continue;
        };
        let is_host = Some(*player_entity) == host_entity;
        let is_me = Some(*player_entity) == my_entity;
        let color = PLAYER_COLORS
            .get(*slot as usize)
            .copied()
            .unwrap_or(Color::WHITE);

        let label = match (is_host, is_me) {
            (true, true) => format!("Player {} [HOST] (You)", slot + 1),
            (true, false) => format!("Player {} [HOST]", slot + 1),
            (false, true) => format!("Player {} (You)", slot + 1),
            (false, false) => format!("Player {}", slot + 1),
        };
        let _ = player;

        let row = commands
            .spawn((
                LobbyPlayerRow,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    width: Val::Percent(100.0),
                    height: Val::Px(28.0),
                    ..default()
                },
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        width: Val::Px(16.0),
                        height: Val::Px(16.0),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(color),
                ));
                row.spawn((
                    Text::new(label),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            })
            .id();

        commands.entity(list_entity).add_child(row);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_start_game_click(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<(), With<StartGameButton>>,
    turn_state: Query<&TurnState>,
    players: Query<(), With<Player>>,
    controller: Res<Controller>,
    defeated: Query<(), With<DefeatedPlayer>>,
    victorious: Query<(), With<VictoriousPlayer>>,
) {
    if !buttons.contains(click.entity) {
        return;
    }
    if local_player_game_over(&controller, &defeated, &victorious) {
        return;
    }
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Lobby {
        return;
    }
    if players.iter().count() < 2 {
        return;
    }
    commands.client_trigger(StartGame);
}
