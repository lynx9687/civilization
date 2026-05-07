use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::unit_definition::{UnitRegistry, UnitTypeId};

/// Stable identifier for a production recipe known by both client and server.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProductionRecipeId(pub u16);

impl ProductionRecipeId {
    pub fn unit(type_id: UnitTypeId) -> Self {
        Self(type_id.0 as u16)
    }
}

/// Server-authoritative recipe copied into city production when selected.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionRecipe {
    pub id: ProductionRecipeId,
    pub cost: u32,
    pub output: ProductionOutput,
}

/// Thing created when a production recipe completes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductionOutput {
    Unit { type_id: UnitTypeId },
}

/// Replicated production state for a city.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct CityProduction {
    pub recipe: Option<ProductionRecipe>,
    pub accumulated: u32,
}

/// Registry of all currently producible recipes.
#[derive(Resource, Default, Debug)]
pub struct RecipeRegistry {
    pub recipes: HashMap<ProductionRecipeId, ProductionRecipe>,
}

impl RecipeRegistry {
    pub fn get(&self, id: &ProductionRecipeId) -> Option<&ProductionRecipe> {
        self.recipes.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ProductionRecipeId, &ProductionRecipe)> {
        self.recipes.iter()
    }

    pub fn from_unit_registry(units: &UnitRegistry) -> Self {
        let recipes = units
            .definitions
            .iter()
            .map(|(type_id, definition)| {
                let id = ProductionRecipeId::unit(*type_id);
                (
                    id,
                    ProductionRecipe {
                        id,
                        cost: definition.production_cost,
                        output: ProductionOutput::Unit { type_id: *type_id },
                    },
                )
            })
            .collect();

        Self { recipes }
    }
}

/// Startup system that derives production recipes from loaded definition registries.
pub fn load_recipe_registry(mut commands: Commands, units: Res<UnitRegistry>) {
    let registry = RecipeRegistry::from_unit_registry(&units);
    println!("Loaded {} production recipes", registry.recipes.len());
    commands.insert_resource(registry);
}
