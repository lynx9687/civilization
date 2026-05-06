use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;
use serde::{Deserialize, Serialize};

/// Marker for hex tile entities.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct HexTile;

/// Player identity — color_index is unique per player, assigned by server.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug)]
#[require(Replicated)]
pub struct Player {
    pub color_index: u8,
    pub gold: i32,
}

/// Replicated turn state — lives on a single entity spawned by the server.
/// We use entity rather than resource because entities can be autmatically replicated by replicon
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[require(Replicated)]
pub struct TurnState {
    pub phase: TurnPhase,
    pub turn_number: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TurnPhase {
    WaitingForPlayers,
    Accepting,
}

/// Player colors for rendering. Index by Player::color_index.
pub const PLAYER_COLORS: [Color; 8] = [
    Color::srgb(0.9, 0.2, 0.2), // red
    Color::srgb(0.2, 0.4, 0.9), // blue
    Color::srgb(0.2, 0.8, 0.2), // green
    Color::srgb(0.9, 0.9, 0.2), // yellow
    Color::srgb(0.9, 0.2, 0.9), // magenta
    Color::srgb(0.2, 0.9, 0.9), // cyan
    Color::srgb(0.9, 0.6, 0.2), // orange
    Color::srgb(0.6, 0.2, 0.9), // purple
];

pub fn player_color(index: u8) -> Color {
    PLAYER_COLORS[index as usize % PLAYER_COLORS.len()]
}
