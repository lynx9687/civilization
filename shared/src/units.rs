use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::hex::HexPosition;

// Tracks the player owning a unit
// I'm not sure if this is the correct way to do it. Needs discussion - Kacper
#[derive(Component, Serialize, Deserialize, Debug)]
pub struct Owner {
    pub player_id: u32,
}

#[derive(Component, Serialize, Deserialize, Debug)]
pub struct ColorIndex(pub u8);

// Entity for units such as warrior/settler
#[derive(Component, Serialize, Deserialize, Debug)]
#[require(Replicated, HexPosition)] //intuitively we want every unit to have an owner but Entity doesn't have default
pub struct Unit {
    pub id: u32,
}

/// Assigns unique ids to players
#[derive(Resource, Default)]
pub struct UnitCounter(u32);

impl UnitCounter {
    pub fn next_id(&mut self) -> u32 {
        let res = self.0;
        self.0 += 1;
        res
    }
}

#[derive(Component, Serialize, Deserialize, Debug)]
pub struct MoveTo {
    pub pos: HexPosition,
}
