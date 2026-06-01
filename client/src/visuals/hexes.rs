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
    /// Placeholder is grass for every terrain; the per-terrain visuals work swaps
    /// in distinct textures/colors here.
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
        // Placeholder: grass for every terrain. Per-terrain visuals replace these.
        terrain: Terrain::ALL
            .iter()
            .map(|_| materials.add(hex_material(default_texture.clone(), Color::WHITE)))
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
