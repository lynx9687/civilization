use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;
use serde::{Deserialize, Serialize};

/// Marker for hex tile entities.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct HexTile;

/// Player identity — color_index is the display slot (0-based), reassigned on
/// disconnect so the lobby list stays contiguous.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug)]
#[require(Replicated)]
pub struct Player {
    pub color_index: u8,
    pub gold: i32,
}

/// Seconds allowed per turn; enforced server-side, displayed client-side.
pub const TURN_DURATION_SECS: u32 = 60;

/// Replicated turn state — lives on a single entity spawned by the server.
/// We use entity rather than resource because entities can be autmatically replicated by replicon
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[require(Replicated)]
pub struct TurnState {
    pub phase: TurnPhase,
    pub turn_number: u32,
    /// Seconds elapsed in the current turn, updated every second by the server.
    /// Replicated so clients can display an accurate countdown even after reconnecting.
    pub turn_elapsed_secs: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub enum TurnPhase {
    #[default]
    Lobby,
    WaitingForPlayers,
    Accepting,
}

/// Marker placed on the Player entity of the current host.
/// Replicated so clients can identify the host without extra events.
#[derive(Component, Serialize, Deserialize, Clone, Debug)]
pub struct Host;

/// Marker spawned on a client's entity when they connect during an active game.
/// They wait in the lobby until the current game ends, then get promoted to Player.
/// Replicated so the client can detect its own waiting state.
#[derive(Component, Serialize, Deserialize, Clone, Debug)]
#[require(Replicated)]
pub struct WaitingPlayer;

/// Marker placed on players who have been eliminated from the current game.
/// Replicated so their client can switch to a loss screen and stop interaction.
#[derive(Component, Serialize, Deserialize, Clone, Debug)]
pub struct DefeatedPlayer;

/// Marker placed on the last surviving player in a completed game.
/// Replicated so their client can switch to a victory screen and stop interaction.
#[derive(Component, Serialize, Deserialize, Clone, Debug)]
pub struct VictoriousPlayer;

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
