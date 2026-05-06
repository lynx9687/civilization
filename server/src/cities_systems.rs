use crate::cities::*;
use bevy::prelude::*;
use shared::{
    cities::*,
    components::{HexTile, Player},
    hex::HexPosition,
    tiles::*,
};

/// Applies stored food income and expands borders after population growth during turn resolution
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
                commands.trigger(GrowCity {
                    entity: city_entity,
                });
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

/// Claims every tile inside each city's current border range.
pub fn claim_city_tiles(
    event: On<GrowCity>,
    mut commands: Commands,
    cities: Query<(Entity, &City, &CityOwner, &HexPosition, &CityStats)>,
    tiles: Query<(Entity, &HexPosition, Option<&TileOwner>), With<HexTile>>,
) {
    let Ok((city_entity, _city, owner, city_pos, stats)) = cities.get(event.entity) else {
        return;
    };
    for (tile_entity, tile_pos, tile_owner) in &tiles {
        if tile_owner.is_some_and(|tile_owner| tile_owner.city_entity != city_entity) {
            continue;
        }

        if city_pos.distance(tile_pos) <= stats.border_range {
            commands.entity(tile_entity).insert(TileOwner {
                city_entity,
                player_entity: Some(owner.entity),
            });
        }
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

        stats.food_per_turn = food_per_turn;
        stats.production = production;
        stats.gold_per_turn = gold_per_turn;
    }
}
