use bevy::prelude::*;
use bevy_replicon::prelude::*;

use crate::cities::*;
use crate::components::*;
use crate::events::*;
use crate::hex::HexPosition;
use crate::production::*;
use crate::tiles::*;
use crate::unit_definition::load_unit_registry;
use crate::units::*;

pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<HexPosition>()
            .replicate::<HexTile>()
            .replicate::<Player>()
            .replicate::<Host>()
            .replicate::<WaitingPlayer>()
            .replicate::<TurnState>()
            .replicate::<Unit>()
            .replicate::<Owner>()
            .replicate::<ColorIndex>()
            .replicate::<MoveTo>()
            .replicate::<AttackTarget>()
            .replicate::<City>()
            .replicate::<CityStats>()
            .replicate::<CityProduction>()
            .replicate::<CityOwner>()
            .replicate::<OwnedCities>()
            .replicate::<TileResources>()
            .replicate::<TileOwner>()
            .replicate::<Health>()
            .add_mapped_client_event::<UnitActionEvent>(Channel::Ordered)
            .add_mapped_client_event::<CityActionEvent>(Channel::Ordered)
            .add_client_event::<FinishTurn>(Channel::Ordered)
            .add_client_event::<StartGame>(Channel::Ordered)
            .add_mapped_server_event::<YourPlayer>(Channel::Ordered)
            .add_systems(Startup, (load_unit_registry, load_recipe_registry).chain());
    }
}
