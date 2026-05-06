use bevy::prelude::*;
use shared::{
    cities::{City, CityOwner, CityStats},
    hex::HexPosition,
    tiles::{TileOwner, TileResources},
    units::ColorIndex,
};

pub const DEFAULT_TILE_RESOURCES: TileResources = TileResources {
    food: 2,
    production: 1,
    gold: 1,
};

pub const STARTING_POPULATION: u32 = 1;
pub const STARTING_BORDER_RANGE: i32 = 1;
pub const MAX_BORDER_RANGE: i32 = 3;
pub const POPULATION_PER_BORDER_RANGE: u32 = 3;
pub const FOOD_GROWTH_MULTIPLIER: i32 = 5;

/// Server-only marker for cities whose border claims need refreshing.
#[derive(Component)]
pub struct PendingTileClaim;

/// Spawns a city controlled by `player_id` at the given map tile.
pub fn spawn_city_at_tile(
    commands: &mut Commands,
    pos: HexPosition,
    player_entity: Entity,
    color_index: u8,
) -> Entity {
    commands
        .spawn((
            City,
            CityStats {
                population: STARTING_POPULATION,
                food: 0,
                food_per_turn: 0,
                production: 0,
                gold_per_turn: 0,
                border_range: STARTING_BORDER_RANGE,
            },
            pos,
            CityOwner {
                entity: player_entity,
            },
            ColorIndex(color_index),
            PendingTileClaim,
        ))
        .id()
}

pub fn any_pending_city_claims(cities: Query<(), With<PendingTileClaim>>) -> bool {
    !cities.is_empty()
}

type ChangedTileOwners<'w, 's> = Query<'w, 's, (), Or<(Added<TileOwner>, Changed<TileOwner>)>>;
type ChangedTileResources<'w, 's> =
    Query<'w, 's, (), Or<(Added<TileResources>, Changed<TileResources>)>>;

pub fn any_city_yields_need_recalculation(
    owned_tiles: ChangedTileOwners,
    resource_tiles: ChangedTileResources,
) -> bool {
    !owned_tiles.is_empty() || !resource_tiles.is_empty()
}
