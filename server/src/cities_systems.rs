use crate::cities::*;
use bevy::prelude::*;
use bevy_replicon::prelude::{ClientId, FromClient};
use shared::{
    cities::*,
    components::{DefeatedPlayer, HexTile, Player, TurnPhase, TurnState},
    events::{CityAction, CityActionEvent},
    hex::HexPosition,
    production::{CityProduction, ProductionOutput, RecipeRegistry},
    tiles::*,
    unit_definition::UnitRegistry,
    units::{ColorIndex, Health, Owner, Unit},
};

use crate::players::PlayerMap;

#[derive(EntityEvent)]
pub struct ProductionCompleted {
    pub entity: Entity,
    pub output: ProductionOutput,
}

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

pub fn handle_city_action(
    trigger: On<FromClient<CityActionEvent>>,
    player_map: Res<PlayerMap>,
    recipes: Res<RecipeRegistry>,
    turn_state: Query<&TurnState>,
    mut cities: Query<(&CityOwner, &mut CityProduction), With<City>>,
    defeated: Query<(), With<DefeatedPlayer>>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Accepting {
        return;
    }

    let client_entity = match trigger.client_id {
        ClientId::Client(entity) => entity,
        ClientId::Server => return,
    };
    let Some(player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };
    if defeated.contains(*player_entity) {
        return;
    }

    let Ok((owner, mut production)) = cities.get_mut(trigger.message.city) else {
        return;
    };
    if owner.entity != *player_entity {
        return;
    }

    match trigger.message.action {
        CityAction::SetProduction { recipe_id } => {
            let Some(recipe) = recipes.get(&recipe_id) else {
                println!("Rejected city production: unknown recipe {recipe_id:?}");
                return;
            };
            production.recipe = Some(*recipe);
            //restore production if recipe is same as last turn
            if production.recipe == production.prev_recipe {
                production.accumulated = production.prev_accumulated;
            } else {
                //don't waste production by allowing it to overflow
                production.accumulated = production.overflown_production;
            }
        }
        CityAction::ClearProduction => {
            production.recipe = None;
            production.accumulated = 0;
        }
    }
}

/// Advances selected city production and emits a completion event for finished recipes.
pub fn advance_city_production(
    mut commands: Commands,
    mut cities: Query<(Entity, &CityStats, &mut CityProduction), With<City>>,
) {
    for (entity, stats, mut production) in &mut cities {
        if let Some(recipe) = production.recipe {
            production.overflown_production = 0;
            production.accumulated = production
                .accumulated
                .saturating_add(stats.production.max(0) as u32);
            if production.accumulated >= recipe.cost {
                production.overflown_production = production.accumulated - recipe.cost;
                production.recipe = None;
                production.accumulated = 0;
                commands.trigger(ProductionCompleted {
                    entity,
                    output: recipe.output,
                });
            }
        } else {
            production.accumulated = 0;
            production.overflown_production = 0;
        }
        production.prev_recipe = production.recipe;
        production.prev_accumulated = production.accumulated;
    }
}

pub fn complete_unit_production(
    event: On<ProductionCompleted>,
    mut commands: Commands,
    cities: Query<(&CityOwner, &HexPosition, &ColorIndex), With<City>>,
    registry: Res<UnitRegistry>,
) {
    let ProductionOutput::Unit { type_id } = event.output;
    let Ok((owner, pos, color)) = cities.get(event.entity) else {
        return;
    };
    let Some(definition) = registry.get(&type_id) else {
        println!("Completed production for unknown unit type {type_id:?}");
        return;
    };

    commands.spawn((
        Unit { type_id },
        *pos,
        Owner(owner.entity),
        ColorIndex(color.0),
        Health::full(definition.hp),
    ));
}

/// Claims every tile inside each city's current border range.
pub fn claim_city_tiles(
    event: On<GrowCity>,
    mut commands: Commands,
    cities: Query<(Entity, &CityOwner, &HexPosition, &CityStats)>,
    tiles: Query<(Entity, &HexPosition, Option<&TileOwner>), With<HexTile>>,
) {
    let Ok((city_entity, owner, city_pos, stats)) = cities.get(event.entity) else {
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

/// Recomputes city income from currently owned tiles.
pub fn recalculate_city_yields(
    mut cities: Query<(&OwnedTiles, &mut CityStats), With<City>>,
    all_tiles: Query<&TileResources, With<HexTile>>,
) {
    for (tiles, mut stats) in &mut cities {
        let mut food_per_turn = 0;
        let mut production = 0;
        let mut gold_per_turn = 0;

        for tile_entity in tiles.collection() {
            if let Ok(resources) = all_tiles.get(*tile_entity) {
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
