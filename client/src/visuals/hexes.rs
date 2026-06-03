use bevy::prelude::*;
use shared::{
    components::{HexTile, PLAYER_COLORS},
    hex::{HexPosition, hex_to_pixel},
    terrain::Terrain,
};

use crate::HEX_SIZE;

const HEX_TINT_STRENGTH: f32 = 1.0;

/// Handles to shared hex materials for highlighting.
#[derive(Resource)]
pub struct HexMaterials {
    pub default: Handle<ColorMaterial>,
    pub hovered: Handle<ColorMaterial>,
    pub valid_move: Handle<ColorMaterial>,
    pub claimed: Vec<Handle<ColorMaterial>>,
    pub valid_attack: Handle<ColorMaterial>,
    /// Base material per terrain, indexed by `terrain as usize` (see `Terrain::ALL`).
    /// Each terrain gets a distinct, readable base (see `terrain_material`).
    pub terrain: Vec<Handle<ColorMaterial>>,
}

impl HexMaterials {
    /// The unhighlighted base material for a tile's terrain.
    pub fn terrain_material(&self, terrain: Terrain) -> Handle<ColorMaterial> {
        self.terrain[terrain as usize].clone()
    }
}

pub fn setup_hex_materials(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let default_texture = asset_server.load("textures/tiles/grass.png");
    let hex_materials = HexMaterials {
        default: materials.add(hex_material(default_texture.clone(), Color::WHITE)),
        hovered: materials.add(tinted_hex_material(
            default_texture.clone(),
            Color::srgb(0.8, 0.8, 0.9),
        )),
        valid_move: materials.add(tinted_hex_material(
            default_texture.clone(),
            Color::srgb(0.4, 0.8, 0.4),
        )),
        claimed: PLAYER_COLORS
            .iter()
            .map(|color| materials.add(tinted_hex_material(default_texture.clone(), *color)))
            .collect(),
        valid_attack: materials.add(tinted_hex_material(
            default_texture.clone(),
            Color::srgb(0.8, 0.3, 0.3),
        )),
        // Distinct base material per terrain, indexed by `terrain as usize`.
        // Iterating `Terrain::ALL` keeps index == discriminant reorder-safe.
        terrain: Terrain::ALL
            .iter()
            .map(|t| materials.add(terrain_material(default_texture.clone(), *t)))
            .collect(),
    };
    commands.insert_resource(hex_materials);
}

pub fn spawn_hex_visuals(
    tiles: Query<(Entity, &HexPosition, Option<&Terrain>), Added<HexTile>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    hex_materials: Res<HexMaterials>,
) {
    for (entity, pos, terrain) in &tiles {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let base = terrain
            .map(|t| hex_materials.terrain_material(*t))
            .unwrap_or_else(|| hex_materials.default.clone());
        commands.entity(entity).insert((
            Mesh2d(meshes.add(RegularPolygon::new(HEX_SIZE * 0.95, 6))),
            MeshMaterial2d(base),
            Transform::from_xyz(pixel.x, pixel.y, 0.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_6)),
        ));
    }
}

/// Distinct base material for a terrain. The grass texture is green-dominant
/// (avg ~RGB 111,147,36), so a multiplicative tint can only reach the greens:
/// grassland is the texture untouched, hill an olive/tan, forest a darker green.
/// Gray (mountain) and blue (water) aren't reachable by multiplying a green
/// texture, so they're flat fills with no texture to guarantee the hue.
fn terrain_material(texture: Handle<Image>, terrain: Terrain) -> ColorMaterial {
    match terrain {
        // Texture as-is: the grass png already reads as grassland.
        Terrain::Grassland => hex_material(texture, Color::WHITE),
        // Lift red/blue, hold green to pull the yellow-green toward olive/tan.
        Terrain::Hill => hex_material(texture, Color::srgb(1.3, 0.95, 1.2)),
        // Darken and bias green so it reads as a deeper forest green.
        Terrain::Forest => hex_material(texture, Color::srgb(0.4, 0.7, 0.3)),
        // Flat fills: a green texture can't multiply down to gray/blue.
        Terrain::Mountain => flat_material(Color::srgb(0.5, 0.5, 0.5)),
        Terrain::Water => flat_material(Color::srgb(0.2, 0.45, 0.85)),
    }
}

/// Untextured solid color — used where a tint of the grass texture can't reach
/// the target hue (mountain gray, water blue).
fn flat_material(color: Color) -> ColorMaterial {
    ColorMaterial {
        color,
        texture: None,
        ..default()
    }
}

fn tinted_hex_material(texture: Handle<Image>, color: Color) -> ColorMaterial {
    let srgba_color = color.to_srgba();
    let soften = |channel| 0.5 + channel * HEX_TINT_STRENGTH;
    hex_material(
        texture,
        Color::srgb(
            soften(srgba_color.red),
            soften(srgba_color.green),
            soften(srgba_color.blue),
        ),
    )
}

fn hex_material(texture: Handle<Image>, color: Color) -> ColorMaterial {
    ColorMaterial {
        color,
        texture: Some(texture),
        ..default()
    }
}
