// Types and function are stubs; future tasks will wire them into ECS systems.
#![allow(dead_code)]

use bevy::prelude::Entity;
use shared::hex::HexPosition;
use std::collections::{HashMap, HashSet};

/// One row per live unit, gathered by the wrapper system before calling the algorithm.
#[derive(Clone, Debug)]
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
    /// No movement this turn.
    Stationary,
    /// Move to this destination. Triggers melee combat if an enemy ends up there too.
    MoveTo(HexPosition),
}

#[derive(Default, Debug)]
pub struct CombatDeltas {
    pub hp_changes: HashMap<Entity, i32>,
    pub final_positions: HashMap<Entity, HexPosition>,
    pub deaths: HashSet<Entity>,
}

/// Stub. Fleshed out across the next tasks.
pub fn resolve_movement_pure(_units: Vec<UnitSnapshot>) -> CombatDeltas {
    CombatDeltas::default()
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
}
