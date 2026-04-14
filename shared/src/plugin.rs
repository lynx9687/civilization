use bevy::prelude::*;
use bevy_replicon::prelude::*;

use crate::components::*;
use crate::hex::HexPosition;

pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<HexPosition>()
            .replicate::<HexTile>()
            .replicate::<Player>()
            .replicate::<TurnState>()
            .add_client_event::<MoveAction>(Channel::Ordered)
            .add_server_event::<YourPlayer>(Channel::Ordered);
    }
}
