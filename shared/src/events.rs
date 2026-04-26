use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Client-to-server event: player wants to move unit to target hex.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct MoveAction {
    pub unit: Entity,
    pub target: crate::hex::HexPosition,
}

/// Server-to-client event: tells a client which player is theirs.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct YourPlayer {
    pub color_index: u8,
}

/// Client-to-server event: player finished his turn
#[derive(Event, Serialize, Deserialize)]
pub struct FinishTurn;