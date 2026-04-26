use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::{components::*, hex::HexPosition, events::*};

use crate::GRID_RADIUS;
use crate::players::PlayerMap;

/// Collects submitted moves during the Accepting phase, then drains them all at once to resolve the turn.
/// Maps player entity → target hex.
#[derive(Resource, Default)]
pub struct PendingMoves {
    pub moves: HashMap<Entity, HexPosition>,
}

pub fn update_turn_phase(
    players: Query<(), With<Player>>,
    mut turn_state: Query<&mut TurnState>,
    mut pending_moves: ResMut<PendingMoves>,
) {
    let count = players.iter().count();
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };

    if count < 2 {
        if state.phase != TurnPhase::WaitingForPlayers {
            state.phase = TurnPhase::WaitingForPlayers;
            pending_moves.moves.clear();
            println!("Not enough players ({count}), waiting...");
        }
    } else if state.phase == TurnPhase::WaitingForPlayers {
        state.phase = TurnPhase::Accepting;
        println!(
            "Enough players ({count}), accepting moves for turn {}",
            state.turn_number
        );
    }
}

pub fn handle_move(
    trigger: On<FromClient<MoveAction>>,
    player_map: Res<PlayerMap>,
    players: Query<&HexPosition, With<Player>>,
    mut pending_moves: ResMut<PendingMoves>,
    turn_state: Query<&TurnState>,
) {
    let client_entity = match trigger.client_id {
        ClientId::Client(entity) => entity,
        ClientId::Server => return,
    };
    let target = trigger.message.target;

    let Some(&player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };

    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    if pending_moves.moves.contains_key(&player_entity) {
        return; // already submitted this turn
    }

    let Ok(current_pos) = players.get(player_entity) else {
        return;
    };

    if !current_pos.is_neighbor(&target) {
        println!(
            "Rejected move: {:?} is not a neighbor of {:?}",
            target, current_pos
        );
        return;
    }
    if !target.in_bounds(GRID_RADIUS) {
        println!("Rejected move: {:?} is out of bounds", target);
        return;
    }

    pending_moves.moves.insert(player_entity, target);
    println!(
        "Move accepted: player {player_entity} -> {:?} ({}/?)",
        target,
        pending_moves.moves.len()
    );
}

pub fn resolve_turn(
    mut pending_moves: ResMut<PendingMoves>,
    players: Query<Entity, With<Player>>,
    mut positions: Query<&mut HexPosition, With<Player>>,
    mut turn_state: Query<&mut TurnState>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let player_count = players.iter().count();
    if player_count < 2 || pending_moves.moves.len() < player_count {
        return;
    }

    // Apply all moves simultaneously
    for (entity, target) in pending_moves.moves.drain() {
        if let Ok(mut pos) = positions.get_mut(entity) {
            *pos = target;
        }
    }

    // Advance turn
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };
    state.turn_number += 1;
    println!("Turn resolved! Now on turn {}", state.turn_number);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pending_moves_tracking() {
        let mut pending = PendingMoves::default();
        let entity = Entity::from_bits(1);
        let target = HexPosition::new(1, 0);

        assert!(!pending.moves.contains_key(&entity));
        pending.moves.insert(entity, target);
        assert!(pending.moves.contains_key(&entity));
        assert_eq!(pending.moves.len(), 1);

        pending.moves.drain();
        assert_eq!(pending.moves.len(), 0);
    }
}
