use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Overall map scale. The server turns this into a base radius and grows it with
/// the player count (see `server/src/map_gen.rs`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MapSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// Host-chosen map-generation settings.
///
/// Lives in `shared` so the lobby (client) can build it and send it over the wire,
/// and the server can store it as a `Resource` and read it at game start. The knob
/// fields are normalized `0.0..=1.0` weights the generator interprets.
#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MapSettings {
    pub size: MapSize,
    /// Fixed seed for reproducible maps; `None` means generate from entropy.
    pub seed: Option<u64>,
    /// Weight of hills/mountains.
    pub hilliness: f32,
    /// Weight of forest cover.
    pub forest: f32,
    /// Weight of water (kept toward the map edge so the land stays connected).
    pub water: f32,
}

impl Default for MapSettings {
    fn default() -> Self {
        MapSettings {
            size: MapSize::Medium,
            seed: None,
            hilliness: 0.3,
            forest: 0.3,
            water: 0.2,
        }
    }
}
