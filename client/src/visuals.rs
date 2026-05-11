mod cities;
mod hexes;
mod units;

use crate::input::update_hex_highlights;
use bevy::prelude::*;

pub use cities::{spawn_city_visuals, update_city_visuals};
pub use hexes::{HexMaterials, setup_hex_materials, spawn_hex_visuals};
pub use units::{spawn_unit_visuals, update_unit_health_bars, update_unit_positions};

pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_hex_visuals,
                spawn_unit_visuals,
                spawn_city_visuals,
                update_unit_positions,
                update_hex_highlights,
            ),
        );
    }
}
