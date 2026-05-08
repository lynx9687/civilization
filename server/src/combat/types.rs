use bevy::prelude::Entity;
use shared::hex::HexPosition;
use std::collections::{HashMap, HashSet};

/// One row per live unit, gathered by the wrapper system before calling the algorithm.
#[derive(Clone, Debug)]
pub struct UnitSnapshot {
    pub entity: Entity,
    // owner, max_hp, and attack_range are captured for future algorithm expansions
    // (e.g. faction-aware combat, morale, extended-range melee) but not yet read.
    #[allow(dead_code)]
    pub owner: Entity,
    pub hp: i32,
    #[allow(dead_code)]
    pub max_hp: u32,
    pub attack_damage: u32,
    #[allow(dead_code)]
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
