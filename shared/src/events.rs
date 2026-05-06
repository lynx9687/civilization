use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::hex::HexPosition;
use crate::unit_definition::UnitVerb;

/// Single client-to-server event covering all per-unit verbs.
/// `unit` is mapped by replicon between client-side and server-side Entity ids.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct UnitActionEvent {
    pub unit: Entity,
    pub action: UnitAction,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum UnitAction {
    Move { target: HexPosition },
    Attack { target: HexPosition },
    Fortify,
    Build { project: String },
    Skip,
}

impl UnitAction {
    pub fn verb(&self) -> UnitVerb {
        match self {
            UnitAction::Move { .. } => UnitVerb::Move,
            UnitAction::Attack { .. } => UnitVerb::Attack,
            UnitAction::Fortify => UnitVerb::Fortify,
            UnitAction::Build { .. } => UnitVerb::Build,
            UnitAction::Skip => UnitVerb::Skip,
        }
    }
}

// required by replicon's add_mapped_client_event for cross-side Entity remap
impl MapEntities for UnitActionEvent {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.unit = mapper.get_mapped(self.unit);
    }
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
