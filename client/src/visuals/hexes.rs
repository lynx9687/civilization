use bevy::prelude::*;
use shared::{
    components::{HexTile, PLAYER_COLORS},
    hex::{HexPosition, hex_to_pixel},
    terrain::Terrain,
};

use crate::HEX_SIZE;

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
};

const HEX_TINT_STRENGTH: f32 = 1.0;
const HEX_MESH_ROTATION: f32 = std::f32::consts::FRAC_PI_6;

/// Handles to shared hex materials for highlighting.
#[derive(Resource)]
pub struct HexMaterials {
    pub default: Handle<ColorMaterial>,
    pub hovered: Handle<ColorMaterial>,
    pub valid_move: Handle<ColorMaterial>,
    pub claimed: Vec<Handle<ColorMaterial>>,
    pub valid_attack: Handle<ColorMaterial>,
    /// Base material per terrain, indexed by `terrain as usize` (see `Terrain::ALL`).
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
        // Distinct texture per terrain, indexed by `terrain as usize`.
        // Iterating `Terrain::ALL` keeps index == discriminant reorder-safe.
        terrain: Terrain::ALL
            .iter()
            .map(|t| {
                materials.add(hex_material(
                    asset_server.load(terrain_texture_path(*t)),
                    Color::WHITE,
                ))
            })
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
            Mesh2d(meshes.add(hex_mesh(HEX_SIZE * 0.95))),
            MeshMaterial2d(base),
            Transform::from_xyz(pixel.x, pixel.y, 0.0),
        ));
    }
}

/// Creates the flat-top hex geometry directly so terrain textures stay aligned
/// with the world instead of rotating with the entity transform.
fn hex_mesh(radius: f32) -> Mesh {
    let mut positions = Vec::with_capacity(7);
    let mut normals = Vec::with_capacity(7);
    let mut uvs = Vec::with_capacity(7);
    let mut indices = Vec::with_capacity(18);

    positions.push([0.0, 0.0, 0.0]);
    normals.push([0.0, 0.0, 1.0]);
    uvs.push([0.5, 0.5]);

    for i in 0..6 {
        let angle = std::f32::consts::FRAC_PI_2
            + HEX_MESH_ROTATION
            + i as f32 * std::f32::consts::TAU / 6.0;
        let (sin, cos) = angle.sin_cos();
        let x = cos * radius;
        let y = sin * radius;

        positions.push([x, y, 0.0]);
        normals.push([0.0, 0.0, 1.0]);
        uvs.push([(x / radius + 1.0) * 0.5, 1.0 - (y / radius + 1.0) * 0.5]);
    }

    for i in 1..=6 {
        indices.extend_from_slice(&[0, i, if i == 6 { 1 } else { i + 1 }]);
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn terrain_texture_path(terrain: Terrain) -> &'static str {
    match terrain {
        Terrain::Grassland => "textures/tiles/grass.png",
        Terrain::Hill => "textures/tiles/hill.png",
        Terrain::Forest => "textures/tiles/forest.png",
        Terrain::Mountain => "textures/tiles/mountain.png",
        Terrain::Water => "textures/tiles/water.png",
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
