use bevy::prelude::*;
use shared::{
    cities::{City, CityStats},
    components::{HexTile, Player},
    hex::HexPosition,
    tiles::{TileOwner, TileResources},
    units::{ColorIndex, Owner},
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
    player_id: u32,
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
            Owner { player_id },
            ColorIndex(color_index),
        ))
        .id()
}

/// Claims every tile inside each city's current border range.
pub fn claim_city_tiles(
    mut commands: Commands,
    cities: Query<(&City, &Owner, &HexPosition, &CityStats), With<City>>,
    tiles: Query<(Entity, &HexPosition, Option<&TileOwner>), With<HexTile>>,
) {
    for (city, owner, city_pos, stats) in &cities {
        for (tile_entity, tile_pos, tile_owner) in &tiles {
            if tile_owner.is_some_and(|tile_owner| tile_owner.city_id != city.id) {
                continue;
            }

            if city_pos.distance(tile_pos) <= stats.border_range {
                commands.entity(tile_entity).insert(TileOwner {
                    player_id: owner.player_id,
                    city_id: city.id,
                });
            }
        }
    }
}

/// Rebuilds city income from currently owned tiles.
pub fn recalculate_city_yields(
    mut cities: Query<(&City, &mut CityStats), With<City>>,
    tiles: Query<(&TileOwner, &TileResources), With<HexTile>>,
) {
    for (city, mut stats) in &mut cities {
        let mut food_per_turn = 0;
        let mut production = 0;
        let mut gold_per_turn = 0;

        for (tile_owner, resources) in &tiles {
            if tile_owner.city_id == city.id {
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
pub fn grow_city_population(mut cities: Query<&mut CityStats, With<City>>) {
    for mut stats in &mut cities {
        stats.food += stats.food_per_turn;

        loop {
            let growth_threshold = stats.population as i32 * FOOD_GROWTH_MULTIPLIER;
            if stats.food < growth_threshold {
                break;
            }

            stats.food -= growth_threshold;
            stats.population += 1;

            let border_range = 1 + ((stats.population - 1) / POPULATION_PER_BORDER_RANGE) as i32;
            stats.border_range = border_range.clamp(STARTING_BORDER_RANGE, MAX_BORDER_RANGE);
        }
    }
}

/// Adds each city's gold income to its owning player.
pub fn grant_city_gold(
    cities: Query<(&Owner, &CityStats), With<City>>,
    mut players: Query<&mut Player>,
) {
    for (owner, stats) in &cities {
        let Some(mut player) = players
            .iter_mut()
            .find(|player| player.player_id == owner.player_id)
        else {
            continue;
        };
        player.gold += stats.gold_per_turn;
    }
}
