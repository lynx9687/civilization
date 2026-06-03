use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::cities::{City, CityOwner};
use shared::events::*;
use shared::{components::*, hex::HexPosition, units::*};

use crate::turn::{PlayerState, PlayerTurnState};

/// Maps ConnectedClient entity → Player entity.
/// `join_order` holds client entities in strict connection order so that
/// host reassignment always picks the oldest remaining connected player.
#[derive(Resource, Default)]
pub struct PlayerMap {
    pub client_to_player: HashMap<Entity, Entity>,
    pub join_order: Vec<Entity>,
}

#[derive(SystemParam)]
pub struct NewPlayerSetup<'w> {
    player_map: ResMut<'w, PlayerMap>,
    player_state: ResMut<'w, PlayerState>,
}

pub fn handle_new_clients(
    new_clients: Query<Entity, Added<AuthorizedClient>>,
    mut commands: Commands,
    mut setup: NewPlayerSetup,
    hosts: Query<(), With<Host>>,
    turn_state: Query<&TurnState>,
) {
    let is_lobby = turn_state
        .single()
        .map(|s| s.phase == TurnPhase::Lobby)
        .unwrap_or(true); // treat unknown state as lobby (startup race)

    // Prevents two clients joining in the same frame from both becoming host.
    let mut host_assigned_this_frame = false;

    for client_entity in &new_clients {
        if !is_lobby {
            // Game in progress: place client in a waiting room.
            // They get a minimal WaitingPlayer entity — no units, no turn slot.
            // promote_waiting_players will upgrade them when the game ends.
            let waiting_entity = commands.spawn(WaitingPlayer).id();

            setup
                .player_map
                .client_to_player
                .insert(client_entity, waiting_entity);
            setup.player_map.join_order.push(client_entity);

            let client_id = ClientId::Client(client_entity);
            commands.server_trigger(ToClients {
                mode: SendMode::Direct(client_id),
                message: YourPlayer {
                    player_entity: waiting_entity,
                },
            });

            println!("Client joined during active game (waiting room), entity: {waiting_entity}");
            continue;
        }

        // Normal lobby join: full player setup.
        let color_index = setup.player_map.join_order.len() as u8;
        let player_entity = commands
            .spawn((
                Player {
                    color_index,
                    gold: 0,
                },
                HexPosition::new(0, 0),
            ))
            .id();

        if !host_assigned_this_frame && hosts.is_empty() {
            commands.entity(player_entity).insert(Host);
            host_assigned_this_frame = true;
            println!("Player {player_entity} is HOST");
        }

        setup
            .player_map
            .client_to_player
            .insert(client_entity, player_entity);
        setup.player_map.join_order.push(client_entity);

        setup
            .player_state
            .turn
            .insert(client_entity, PlayerTurnState::InProgress);

        let client_id = ClientId::Client(client_entity);
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(client_id),
            message: YourPlayer { player_entity },
        });

        // Units are placed at game start (see server::map_gen::generate_map_on_start),
        // once the host's map settings and the final player count are known.
        println!("Player joined (color {color_index}), entity: {player_entity}");
    }
}

/// Promotes WaitingPlayer entities to full Player entities (with units) when the
/// game ends and the phase resets to Lobby.  Runs every frame but is a no-op
/// unless there are waiting players AND the phase is Lobby.
pub fn promote_waiting_players(
    turn_state: Query<&TurnState>,
    waiting: Query<Entity, With<WaitingPlayer>>,
    player_map: Res<PlayerMap>,
    mut player_state: ResMut<PlayerState>,
    mut commands: Commands,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Lobby {
        return;
    }
    if waiting.is_empty() {
        return;
    }

    for (idx, &client_entity) in player_map.join_order.iter().enumerate() {
        let Some(&player_entity) = player_map.client_to_player.get(&client_entity) else {
            continue;
        };
        if !waiting.contains(player_entity) {
            continue;
        }

        let color_index = idx as u8;
        commands
            .entity(player_entity)
            .remove::<WaitingPlayer>()
            .insert(Player {
                color_index,
                gold: 0,
            });

        player_state
            .turn
            .insert(client_entity, PlayerTurnState::InProgress);

        // Units for promoted players are placed at the next game start, same as
        // for fresh lobby joins (see server::map_gen::generate_map_on_start).
        println!("Promoted WaitingPlayer {player_entity} to Player (color_index {color_index})");
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn handle_disconnects(
    mut disconnected: RemovedComponents<ConnectedClient>,
    mut player_map: ResMut<PlayerMap>,
    mut commands: Commands,
    mut player_state: ResMut<PlayerState>,
    host_check: Query<(), With<Host>>,
    mut players_query: Query<&mut Player>,
    mut unit_colors: Query<(&Owner, &mut ColorIndex), (With<Unit>, Without<City>)>,
    mut city_colors: Query<(&CityOwner, &mut ColorIndex), (With<City>, Without<Unit>)>,
) {
    for client_entity in disconnected.read() {
        let prev_state = player_state.turn.remove(&client_entity);
        if prev_state.is_some_and(|state| state == PlayerTurnState::Finished) {
            player_state.finished_cnt -= 1;
        }
        if let Some(player_entity) = player_map.client_to_player.remove(&client_entity) {
            // Keep join_order in sync; do this before the host-reassignment check
            // so that join_order.first() already reflects the remaining players.
            player_map.join_order.retain(|&e| e != client_entity);

            let was_host = host_check.contains(player_entity);
            commands.entity(player_entity).despawn();

            if was_host {
                // Oldest remaining connected player (join_order[0]) becomes host.
                if let Some(&oldest_client) = player_map.join_order.first()
                    && let Some(&next_player) = player_map.client_to_player.get(&oldest_client)
                {
                    commands.entity(next_player).insert(Host);
                    println!("Host transferred to {next_player}");
                }
            }

            println!("Player disconnected, despawned {player_entity}");
        }
    }

    // Reassign color indices so the lobby list stays contiguous after any disconnect.
    // Collect (player_entity → new_color) first so we can update units/cities in the same pass.
    let mut color_map: HashMap<Entity, u8> = HashMap::new();
    for (idx, &client_entity) in player_map.join_order.iter().enumerate() {
        if let Some(&player_entity) = player_map.client_to_player.get(&client_entity)
            && let Ok(mut player) = players_query.get_mut(player_entity)
        {
            player.color_index = idx as u8;
            color_map.insert(player_entity, idx as u8);
        }
    }
    // Keep ColorIndex in sync so in-game colors match lobby color indices.
    for (owner, mut color) in &mut unit_colors {
        if let Some(&new_color) = color_map.get(&owner.0) {
            color.0 = new_color;
        }
    }
    for (city_owner, mut color) in &mut city_colors {
        if let Some(&new_color) = color_map.get(&city_owner.entity) {
            color.0 = new_color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// join_order must reflect insertion order and drop removed entries.
    #[test]
    fn join_order_tracks_connection_order_and_shrinks_on_remove() {
        let mut world = bevy::prelude::World::new();
        let c1 = world.spawn_empty().id();
        let c2 = world.spawn_empty().id();
        let c3 = world.spawn_empty().id();
        let p1 = world.spawn_empty().id();
        let p2 = world.spawn_empty().id();
        let p3 = world.spawn_empty().id();

        let mut map = PlayerMap::default();
        map.client_to_player.insert(c1, p1);
        map.join_order.push(c1);
        map.client_to_player.insert(c2, p2);
        map.join_order.push(c2);
        map.client_to_player.insert(c3, p3);
        map.join_order.push(c3);

        // Remove the first (oldest) client — simulates host disconnect.
        map.client_to_player.remove(&c1);
        map.join_order.retain(|&e| e != c1);

        // Oldest remaining must be c2 (second to join), not c3.
        assert_eq!(
            map.join_order.first().copied(),
            Some(c2),
            "oldest remaining client must be c2"
        );
        assert_eq!(
            map.client_to_player.get(map.join_order.first().unwrap()),
            Some(&p2),
            "host must transfer to player entity of c2"
        );
    }
}
