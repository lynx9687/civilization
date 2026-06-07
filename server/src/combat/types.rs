//! Plain data shapes shared between the combat algorithm and its ECS wrapper.
//! `UnitSnapshot` is the per-unit input the wrapper builds from Bevy queries;
//! `CombatDeltas` is the output the algorithm returns and the wrapper applies
//! back to ECS components. Keeping these out of `algorithm.rs` lets the pure
//! function avoid Bevy types beyond `Entity`.

use bevy::prelude::Entity;
use shared::hex::HexPosition;
use std::collections::{HashMap, HashSet};

/// One row per live unit, gathered by the wrapper system before calling the algorithm.
#[derive(Clone, Debug)]
pub struct UnitSnapshot {
    pub entity: Entity,
    // owner distinguishes friendly co-location (a failed move — the mover yields)
    // from enemy co-location (combat) during resolution.
    pub owner: Entity,
    pub hp: i32,
    // max_hp is captured for future algorithm expansions (e.g. morale, healing
    // caps) but not yet read by the resolver.
    #[allow(dead_code)]
    pub max_hp: u32,
    pub attack_damage: u32,
    pub attack_range: u32,
    pub start_pos: HexPosition,
    pub action: ResolveAction,
}

/// One row per city, gathered by the wrapper before combat resolution.
#[derive(Clone, Debug)]
pub struct CitySnapshot {
    pub entity: Entity,
    pub owner: Entity,
    pub hp: i32,
    #[allow(dead_code)]
    pub max_hp: u32,
    pub pos: HexPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveAction {
    /// No movement this turn.
    Stationary,
    /// Move to this destination. Triggers melee combat if an enemy ends up there too.
    MoveTo(HexPosition),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CityCapture {
    pub city: Entity,
    pub by_unit: Entity,
    pub new_owner: Entity,
}

#[derive(Default, Debug)]
pub struct CombatDeltas {
    pub hp_changes: HashMap<Entity, i32>,
    pub city_hp_changes: HashMap<Entity, i32>,
    pub city_captures: Vec<CityCapture>,
    pub final_positions: HashMap<Entity, HexPosition>,
    pub deaths: HashSet<Entity>,
}
