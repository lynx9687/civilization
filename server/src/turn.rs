use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::events::*;
use shared::unit_definition::{
    UnitRegistry, available_verbs, is_within_attack_range, is_within_move_range,
};
use shared::units::*;
use shared::{components::*, hex::HexPosition};

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

#[allow(clippy::too_many_arguments)]
pub fn handle_unit_action(
    trigger: On<FromClient<UnitActionEvent>>,
    mut commands: Commands,
    player_map: Res<PlayerMap>,
    units: Query<(&HexPosition, &Owner, &Unit)>,
    enemy_units: Query<(&HexPosition, &Owner), With<Unit>>,
    turn_state: Query<&TurnState>,
    registry: Res<UnitRegistry>,
) {
    // common-path validation
    let Ok(state) = turn_state.single() else { return; };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let client_entity = match trigger.client_id {
        ClientId::Client(e) => e,
        ClientId::Server => return,
    };
    let Some(player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };

    let entity = trigger.message.unit;
    let Ok((pos, owner, unit)) = units.get(entity) else { return; };
    if owner.0 != *player_entity {
        return;
    }

    let Some(def) = registry.get(&unit.type_id) else {
        println!("Rejected action: unknown unit type {:?}", unit.type_id);
        return;
    };
    let verb = trigger.message.action.verb();
    if !available_verbs(def).contains(&verb) {
        println!("Rejected action: verb {:?} not available for unit type", verb);
        return;
    }

    // single-action invariant: clear any prior queued marker before inserting
    let mut e = commands.entity(entity);
    e.remove::<MoveTo>()
        .remove::<AttackTarget>()
        .remove::<Fortifying>()
        .remove::<BuildProject>()
        .remove::<Skipping>();

    match &trigger.message.action {
        UnitAction::Move { target } => {
            if !is_within_move_range(pos, target, def.move_budget) {
                println!("Rejected move: out of range");
                return;
            }
            if !target.in_bounds(GRID_RADIUS) {
                println!("Rejected move: out of bounds");
                return;
            }
            e.insert(MoveTo { pos: *target });
        }
        UnitAction::Attack { target } => {
            if !is_within_attack_range(pos, target, def.attack_range) {
                println!("Rejected attack: out of range");
                return;
            }
            let enemy_here = enemy_units
                .iter()
                .any(|(p, o)| p == target && o.0 != *player_entity);
            if !enemy_here {
                println!("Rejected attack: no enemy at target");
                return;
            }
            e.insert(AttackTarget { pos: *target });
        }
        UnitAction::Fortify => {
            e.insert(Fortifying);
        }
        UnitAction::Skip => {
            e.insert(Skipping);
        }
        UnitAction::Build { project } => {
            if !def.build_targets.contains(project) {
                println!("Rejected build: project {project:?} not buildable");
                return;
            }
            e.insert(BuildProject {
                name: project.clone(),
            });
        }
    }
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
