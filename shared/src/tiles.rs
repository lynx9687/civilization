use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Resource yields produced by a map tile when claimed by a city.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileResources {
    pub food: i32,
    pub production: i32,
    pub gold: i32,
}

/// City ownership claim over a map tile.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileOwner {
    pub player_id: u32,
    pub city_id: u32,
}
