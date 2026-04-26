use bevy::prelude::*;
use bevy_replicon::prelude::*;

use crate::components::*;
use crate::events::*;
use crate::hex::HexPosition;
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
            .add_client_event::<MoveAction>(Channel::Ordered)
            .add_server_event::<YourPlayer>(Channel::Ordered);
    }
}
