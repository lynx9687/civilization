// Types and function are stubs; future tasks will wire them into ECS systems.

use bevy::prelude::Entity;
use shared::hex::HexPosition;
use std::collections::{HashMap, HashSet};

/// One row per live unit, gathered by the wrapper system before calling the algorithm.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct UnitSnapshot {
    pub entity: Entity,
    pub owner: Entity,
    pub hp: i32,
    pub max_hp: u32,
    pub attack_damage: u32,
    pub attack_range: u32,
    pub start_pos: HexPosition,
    pub action: ResolveAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveAction {
    /// No movement this turn. Constructed in Task 5.
    #[allow(dead_code)]
    Stationary,
    /// Move to this destination. Triggers melee combat if an enemy ends up there too.
    #[allow(dead_code)]
    MoveTo(HexPosition),
}

#[derive(Default, Debug)]
#[allow(dead_code)]
pub struct CombatDeltas {
    pub hp_changes: HashMap<Entity, i32>,
    pub final_positions: HashMap<Entity, HexPosition>,
    pub deaths: HashSet<Entity>,
}

/// Pure combat resolver. Expanded task by task.
#[allow(dead_code)]
pub fn resolve_movement_pure(units: Vec<UnitSnapshot>) -> CombatDeltas {
    // Initial state: every live unit at its desired position.
    let mut positions: HashMap<Entity, HexPosition> = HashMap::new();
    let mut hps: HashMap<Entity, i32> = HashMap::new();
    let initial_hps: HashMap<Entity, i32> = units
        .iter()
        .filter(|u| u.hp > 0)
        .map(|u| (u.entity, u.hp))
        .collect();

    for u in &units {
        if u.hp <= 0 {
            continue;
        }
        let desired = match u.action {
            ResolveAction::Stationary => u.start_pos,
            ResolveAction::MoveTo(t) => t,
        };
        positions.insert(u.entity, desired);
        hps.insert(u.entity, u.hp);
    }

    let deaths: HashSet<Entity> = HashSet::new();

    let hp_changes: HashMap<Entity, i32> = hps
        .iter()
        .filter_map(|(e, &h)| {
            let initial = initial_hps.get(e).copied().unwrap_or(0);
            let delta = h - initial;
            if delta != 0 { Some((*e, delta)) } else { None }
        })
        .collect();

    CombatDeltas {
        hp_changes,
        final_positions: positions,
        deaths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    /// Helper: spawn N empty entities in a fresh World and return their ids.
    /// Lets tests construct distinct Entity values without setting up full ECS state.
    fn fake_entities(n: usize) -> (World, Vec<Entity>) {
        let mut world = World::new();
        let entities: Vec<_> = (0..n).map(|_| world.spawn_empty().id()).collect();
        (world, entities)
    }

    #[test]
    fn empty_input_returns_empty_deltas() {
        let deltas = resolve_movement_pure(vec![]);
        assert!(deltas.hp_changes.is_empty());
        assert!(deltas.final_positions.is_empty());
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn single_mover_lands_at_destination() {
        let (_world, entities) = fake_entities(1);
        let player = Entity::PLACEHOLDER;
        let snapshot = vec![UnitSnapshot {
            entity: entities[0],
            owner: player,
            hp: 10,
            max_hp: 10,
            attack_damage: 4,
            attack_range: 1,
            start_pos: HexPosition::new(0, 0),
            action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
        }];

        let deltas = resolve_movement_pure(snapshot);

        assert_eq!(
            deltas.final_positions.get(&entities[0]),
            Some(&HexPosition::new(1, 0))
        );
        assert!(
            deltas.hp_changes.is_empty(),
            "no damage in a non-conflict move"
        );
        assert!(deltas.deaths.is_empty());
    }
}
