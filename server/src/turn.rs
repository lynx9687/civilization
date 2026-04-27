use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::units::*;
use shared::{components::*, events::*, hex::HexPosition};

use crate::GRID_RADIUS;
use crate::players::PlayerMap;

/// Collects submitted moves during the Accepting phase, then drains them all at once to resolve the turn.
/// Maps player entity → target hex.
#[derive(Resource, Default)]
pub struct PendingMoves {
    pub moves: HashMap<Entity, HexPosition>,
}

/// Represents whether player is still making moves or has finished his turn
#[derive(PartialEq, Eq)]
pub enum PlayerTurnState {
    InProgress,
    Finished,
}

/// Stores information about players
/// TODO: add some methods to automatically update finished_cnt
#[derive(Resource, Default)]
pub struct PlayerState {
    pub turn: HashMap<Entity, PlayerTurnState>,
    pub finished_cnt: i32,
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

pub fn handle_finish_turn(
    trigger: On<FromClient<FinishTurn>>,
    mut player_state: ResMut<PlayerState>,
) {
    let client_entity = match trigger.client_id {
        ClientId::Client(entity) => entity,
        ClientId::Server => return,
    };
    let prev_state = player_state
        .turn
        .insert(client_entity, PlayerTurnState::Finished);
    if prev_state.is_none_or(|state| state == PlayerTurnState::InProgress) {
        player_state.finished_cnt += 1;
    }
    let cnt = player_state.finished_cnt;
    println!("Received finish turn from player {client_entity}. Finished cnt {cnt}");
}

pub fn handle_move(
    trigger: On<FromClient<MoveAction>>,
    mut commands: Commands,
    player_map: Res<PlayerMap>,
    players: Query<&Player>,
    units: Query<(Entity, &Unit, &HexPosition, &Owner)>,
    turn_state: Query<&TurnState>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let client_entity = match trigger.client_id {
        ClientId::Client(entity) => entity,
        ClientId::Server => return,
    };
    let unit_id = trigger.message.unit_id;
    let target = trigger.message.target;
    let Some(player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };

    let Some((unit_entity, _, unit_pos, unit_owner)) =
        units.iter().find(|(_, unit, _, _)| unit.id == unit_id)
    else {
        return;
    };

    let Ok(player) = players.get(*player_entity) else {
        return;
    };

    //make sure player has right to control this unit
    if unit_owner.player_id != player.player_id {
        return;
    };

    //make sure movement is correct
    if !unit_pos.is_neighbor(&target) {
        return;
    };
    if !target.in_bounds(GRID_RADIUS) {
        println!("Rejected move: {target:?} is out of bounds");
        return;
    }

    println!("Accepting valid movement of unit {unit_id} to pos {target:?}");
    //add the correct component MoveTo. Overwrites previous by default
    commands.entity(unit_entity).insert(MoveTo { pos: target });
}

pub fn resolve_turn(
    players: Query<Entity, With<Player>>,
    mut units: Query<(Entity, &MoveTo, &mut HexPosition), With<MoveTo>>,
    mut commands: Commands,
    mut turn_state: Query<&mut TurnState>,
    mut player_state: ResMut<PlayerState>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let player_count = players.iter().count() as i32;
    if player_count < 2 || player_state.finished_cnt < player_count {
        return;
    }

    // Apply all moves simultaneously
    for (entity, move_to, mut pos) in &mut units {
        *pos = move_to.pos;
        commands.entity(entity).remove::<MoveTo>();
    }

    // Advance turn
    let Ok(mut state) = turn_state.single_mut() else {
        return;
    };
    state.turn_number += 1;
    //reset finished players
    player_state.finished_cnt = 0;
    for val in player_state.turn.values_mut() {
        *val = PlayerTurnState::InProgress;
    }
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
