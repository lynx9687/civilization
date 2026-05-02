use bevy::prelude::*;
use bevy_replicon::prelude::*;

use crate::cities::*;
use crate::components::*;
use crate::events::*;
use crate::hex::HexPosition;
use crate::tiles::*;
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
            .replicate::<City>()
            .replicate::<CityStats>()
            .replicate::<TileResources>()
            .replicate::<TileOwner>()
            .replicate::<Health>()
            .add_client_event::<MoveAction>(Channel::Ordered)
            .add_client_event::<FinishTurn>(Channel::Ordered)
            .add_server_event::<YourPlayer>(Channel::Ordered);
    }
}
