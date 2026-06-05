use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::hex::HexPosition;
use crate::map_settings::MapSettings;
use crate::production::ProductionRecipeId;
use crate::unit_definition::UnitVerb;

/// Single client-to-server event covering all per-unit verbs.
/// `unit` is mapped by replicon between client-side and server-side Entity ids.
#[derive(Event, Serialize, Deserialize, MapEntities, Clone, Debug)]
pub struct UnitActionEvent {
    #[entities]
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

/// Server-to-client event: tells a client which player entity is theirs.
#[derive(Event, Serialize, Deserialize, MapEntities, Clone, Debug)]
pub struct YourPlayer {
    #[entities]
    pub player_entity: Entity,
}

/// Client-to-server event: player finished his turn
#[derive(Event, Serialize, Deserialize)]
pub struct FinishTurn;

/// Client-to-server event: host requests the game to start (Lobby → Accepting).
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct StartGame;

/// Client-to-server event: host updates the map-generation settings while in the
/// lobby. The server stores the payload in its `MapSettings` resource, which the
/// generator reads at game start. Only honored from the host during the Lobby phase.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetMapConfig(pub MapSettings);

/// Single client-to-server event covering city-level actions.
/// `city` is mapped by replicon between client-side and server-side Entity ids.
#[derive(Event, Serialize, Deserialize, MapEntities, Clone, Debug)]
pub struct CityActionEvent {
    #[entities]
    pub city: Entity,
    pub action: CityAction,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CityAction {
    SetProduction { recipe_id: ProductionRecipeId },
    ClearProduction,
}
