use bevy::prelude::*;
use bevy_replicon::prelude::*;

use crate::components::*;
use crate::events::*;
use crate::hex::HexPosition;
use crate::unit_definition::load_unit_registry;
use crate::units::*;

pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<HexPosition>()
            .replicate::<HexTile>()
            .replicate::<Player>()
            .replicate::<TurnState>()
            .replicate::<Unit>()
            .replicate::<Owner>()
            .replicate::<ColorIndex>()
            .replicate::<Health>()
            .add_mapped_client_event::<UnitActionEvent>(Channel::Ordered)
            .add_client_event::<FinishTurn>(Channel::Ordered)
            .add_server_event::<YourPlayer>(Channel::Ordered)
            .add_systems(Startup, load_unit_registry);
    }
}
