use bevy::{ecs::entity::MapEntities, prelude::*};
use serde::{Deserialize, Serialize};

/// Client-to-server event: player wants to move unit to target hex.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct MoveAction {
    pub unit_id: u32,
    pub target: crate::hex::HexPosition,
}

/// Server-to-client event: tells a client which player is theirs.
#[derive(Event, Serialize, Deserialize, MapEntities, Clone, Debug)]
pub struct YourPlayer {
    #[entities]
    pub player_entity: Entity,
    pub color_index: u8,
}

/// Client-to-server event: player finished his turn
#[derive(Event, Serialize, Deserialize)]
pub struct FinishTurn;
