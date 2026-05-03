use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::hex::HexPosition;
use crate::unit_definition::UnitTypeId;

#[derive(Component, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    pub current: u32,
    pub max: u32,
}

impl Health {
    pub fn full(max: u32) -> Self {
        Health { current: max, max }
    }
}

// Tracks the player owning a unit. Bevy relationship: owning the player entity.
// `linked_spawn` on `OwnedUnits` cascades despawn — when the player despawns,
// its units despawn automatically. `#[entities]` lets replicon remap the entity
// id from server-space to client-space when replicating.
#[derive(Component, Serialize, Deserialize, Debug)]
#[relationship(relationship_target = OwnedUnits)]
pub struct Owner(#[entities] pub Entity);

#[derive(Component, Default, Debug)]
#[relationship_target(relationship = Owner, linked_spawn)]
pub struct OwnedUnits(Vec<Entity>);

#[derive(Component, Serialize, Deserialize, Debug)]
pub struct ColorIndex(pub u8);

// Entity for units such as warrior/settler
#[derive(Component, Serialize, Deserialize, Debug, Clone, Copy)]
#[require(Replicated, HexPosition)] //intuitively we want every unit to have an owner but Entity doesn't have default
pub struct Unit {
    pub id: u32,
    pub type_id: UnitTypeId,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_full_initializes_at_max() {
        let h = Health::full(10);
        assert_eq!(h.current, 10);
        assert_eq!(h.max, 10);
    }
}
