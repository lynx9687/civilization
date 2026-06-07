mod cities;
mod hexes;
pub mod theme;
mod units;

use crate::input::update_hex_highlights;
use bevy::prelude::*;

pub use cities::{spawn_city_visuals, update_city_health_bars, update_city_visuals};
pub use hexes::{
    HexMaterials, cleanup_ownership_border_visuals, setup_hex_materials, spawn_hex_visuals,
    spawn_ownership_borders, sync_ownership_borders, update_ownership_border_colors,
};
pub(crate) use hexes::{HoverHighlightVisual, hex_mesh};
pub use units::{
    spawn_unit_visuals, update_unit_colors, update_unit_health_bars, update_unit_positions,
};

pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_hex_materials).add_systems(
            Update,
            (
                spawn_hex_visuals,
                spawn_unit_visuals,
                spawn_city_visuals,
                update_unit_positions,
                update_hex_highlights,
                update_city_visuals,
                update_city_health_bars,
                update_unit_health_bars,
                update_unit_colors,
            ),
        );
    }
}
