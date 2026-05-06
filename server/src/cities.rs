use bevy::prelude::*;
use shared::{
    cities::{City, CityOwner, CityStats},
    components::{HexTile, Player},
    hex::HexPosition,
    tiles::{TileOwner, TileResources},
    units::ColorIndex,
};

pub const DEFAULT_TILE_RESOURCES: TileResources = TileResources {
    food: 2,
    production: 1,
    gold: 1,
};

const STARTING_POPULATION: u32 = 1;
const STARTING_BORDER_RANGE: i32 = 1;
const MAX_BORDER_RANGE: i32 = 3;
const POPULATION_PER_BORDER_RANGE: u32 = 3;
const FOOD_GROWTH_MULTIPLIER: i32 = 5;

/// Server-only marker for cities whose border claims need refreshing.
#[derive(Component)]
pub struct PendingTileClaim;

/// Assigns unique ids to cities.
#[derive(Resource, Default)]
pub struct CityCounter(u32);

impl CityCounter {
    pub fn next_id(&mut self) -> u32 {
        let res = self.0;
        self.0 += 1;
        res
    }
}

/// Spawns a city controlled by `player_id` at the given map tile.
pub fn spawn_city_at_tile(
    commands: &mut Commands,
    city_counter: &mut CityCounter,
    pos: HexPosition,
    player_entity: Entity,
    color_index: u8,
) -> Entity {
    let city_id = city_counter.next_id();
    commands
        .spawn((
            City { id: city_id },
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

/// Claims every tile inside each city's current border range.
pub fn claim_city_tiles(
    mut commands: Commands,
    cities: Query<(Entity, &City, &CityOwner, &HexPosition, &CityStats), With<PendingTileClaim>>,
    tiles: Query<(Entity, &HexPosition, Option<&TileOwner>), With<HexTile>>,
) {
    for (city_entity, _city, owner, city_pos, stats) in &cities {
        for (tile_entity, tile_pos, tile_owner) in &tiles {
            if tile_owner.is_some_and(|tile_owner| tile_owner.city_entity != city_entity) {
                continue;
            }

            if city_pos.distance(tile_pos) <= stats.border_range {
                commands.entity(tile_entity).insert(TileOwner {
                    player_entity: owner.entity,
                    city_entity,
                });
            }
        }

        commands.entity(city_entity).remove::<PendingTileClaim>();
    }
}

/// Rebuilds city income from currently owned tiles.
pub fn recalculate_city_yields(
    mut cities: Query<(Entity, &City, &mut CityStats), With<City>>,
    tiles: Query<(&TileOwner, &TileResources), With<HexTile>>,
) {
    for (city_entity, _city, mut stats) in &mut cities {
        let mut food_per_turn = 0;
        let mut production = 0;
        let mut gold_per_turn = 0;

        for (tile_owner, resources) in &tiles {
            if tile_owner.city_entity == city_entity {
                food_per_turn += resources.food;
                production += resources.production;
                gold_per_turn += resources.gold;
            }
        }

        if stats.food_per_turn != food_per_turn {
            stats.food_per_turn = food_per_turn;
        }
        if stats.production != production {
            stats.production = production;
        }
        if stats.gold_per_turn != gold_per_turn {
            stats.gold_per_turn = gold_per_turn;
        }
    }
}

/// Applies stored food income and expands borders after population growth.
pub fn grow_city_population(
    mut commands: Commands,
    mut cities: Query<(Entity, &mut CityStats), With<City>>,
) {
    for (city_entity, mut stats) in &mut cities {
        stats.food += stats.food_per_turn;

        loop {
            let growth_threshold = stats.population as i32 * FOOD_GROWTH_MULTIPLIER;
            if stats.food < growth_threshold {
                break;
            }

            stats.food -= growth_threshold;
            stats.population += 1;

            let old_border_range = stats.border_range;
            let border_range = 1 + ((stats.population - 1) / POPULATION_PER_BORDER_RANGE) as i32;
            stats.border_range = border_range.clamp(STARTING_BORDER_RANGE, MAX_BORDER_RANGE);
            if stats.border_range != old_border_range {
                commands.entity(city_entity).insert(PendingTileClaim);
            }
        }
    }
}

/// Adds each city's gold income to its owning player.
pub fn grant_city_gold(
    cities: Query<(&CityOwner, &CityStats), With<City>>,
    mut players: Query<&mut Player>,
) {
    for (owner, stats) in &cities {
        let Ok(mut player) = players.get_mut(owner.entity) else {
            continue;
        };
        player.gold += stats.gold_per_turn;
    }
}
