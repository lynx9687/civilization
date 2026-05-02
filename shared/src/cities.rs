use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;
use serde::{Deserialize, Serialize};

use crate::hex::HexPosition;

/// Replicated city entity controlled by a player.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug)]
#[require(Replicated, HexPosition)]
pub struct City {
    pub id: u32,
}

/// Replicated economic state for a city.
#[derive(Component, Serialize, Deserialize, Clone, Debug)]
pub struct CityStats {
    pub population: u32,
    pub food: i32,
    pub food_per_turn: i32,
    pub production: i32,
    pub gold_per_turn: i32,
    pub border_range: i32,
}
