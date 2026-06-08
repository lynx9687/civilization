use bevy::prelude::*;
use shared::{
    components::{DefeatedPlayer, HexTile, OwnershipBorder, PLAYER_COLORS, Player},
    hex::{HexPosition, hex_to_pixel},
    terrain::Terrain,
    tiles::TileOwner,
};

use crate::HEX_SIZE;

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
};

const HEX_MESH_ROTATION: f32 = std::f32::consts::FRAC_PI_6;

/// Handles to shared hex materials for highlighting.
#[derive(Resource)]
pub struct HexMaterials {
    pub default: Handle<ColorMaterial>,
    /// Translucent material drawn over a hovered tile without replacing its terrain texture.
    pub hover: Handle<ColorMaterial>,
    pub target_dot: Handle<ColorMaterial>,
    /// Base material per terrain, indexed by `terrain as usize` (see `Terrain::ALL`).
    pub terrain: Vec<Handle<ColorMaterial>>,
    /// Ownership border materials (player colors), indexed by player color_index.
    pub ownership_border: Vec<Handle<ColorMaterial>>,
}

impl HexMaterials {
    /// The unhighlighted base material for a tile's terrain.
    pub fn terrain_material(&self, terrain: Terrain) -> Handle<ColorMaterial> {
        self.terrain[terrain as usize].clone()
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

pub fn setup_hex_materials(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let default_texture = asset_server.load("textures/tiles/grass.png");
    let hex_materials = HexMaterials {
        default: materials.add(hex_material(default_texture.clone(), Color::WHITE)),
        hover: materials.add(ColorMaterial::from(Color::srgba(1.0, 1.0, 1.0, 0.28))),
        target_dot: materials.add(ColorMaterial::from(Color::srgb(0.35, 0.35, 0.35))),
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
        // Ownership border materials (flat player colors, no texture).
        // Indexed by player color_index for easy lookup.
        ownership_border: PLAYER_COLORS
            .iter()
            .map(|color| materials.add(flat_material(*color)))
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
pub(crate) fn hex_mesh(radius: f32) -> Mesh {
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

fn hex_material(texture: Handle<Image>, color: Color) -> ColorMaterial {
    ColorMaterial {
        color,
        texture: Some(texture),
        ..default()
    }
}

#[derive(Component)]
pub(crate) struct OwnershipBorderVisual;

#[derive(Component)]
pub(crate) struct HoverHighlightVisual;

/// Spawn a larger ownership hex behind the real terrain tile.
/// The terrain remains visible in the interior, while the border color shows ownership.
pub fn spawn_ownership_borders(
    tiles: Query<(Entity, &OwnershipBorder), Added<OwnershipBorder>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    hex_materials: Res<HexMaterials>,
) {
    for (entity, ownership) in &tiles {
        let border_material_handle =
            hex_materials.ownership_border[ownership.color_index as usize].clone();
        let border_size = HEX_SIZE * 1.05;
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                OwnershipBorderVisual,
                Mesh2d(meshes.add(hex_mesh(border_size))),
                MeshMaterial2d(border_material_handle),
                Transform::from_xyz(0.0, 0.0, -0.1),
            ));
        });
    }
}

pub fn update_ownership_border_colors(
    changed_ownership: Query<(&OwnershipBorder, &Children), Changed<OwnershipBorder>>,
    mut border_materials: Query<&mut MeshMaterial2d<ColorMaterial>, With<OwnershipBorderVisual>>,
    hex_materials: Res<HexMaterials>,
) {
    for (ownership, children) in &changed_ownership {
        let desired_material =
            hex_materials.ownership_border[ownership.color_index as usize].clone();
        for child in children.iter() {
            if let Ok(mut material) = border_materials.get_mut(child) {
                *material = MeshMaterial2d(desired_material.clone());
            }
        }
    }
}

pub fn cleanup_ownership_border_visuals(
    mut commands: Commands,
    owned_borders: Query<(Entity, &ChildOf), With<OwnershipBorderVisual>>,
    owners: Query<&OwnershipBorder>,
) {
    for (entity, parent) in &owned_borders {
        if owners.get(parent.0).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

type OwnershipBorderSyncQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        Option<&'static TileOwner>,
        Option<&'static OwnershipBorder>,
    ),
    With<HexTile>,
>;

pub fn sync_ownership_borders(
    mut commands: Commands,
    tiles: OwnershipBorderSyncQuery,
    players: Query<&Player, Without<DefeatedPlayer>>,
) {
    for (entity, tile_owner, existing_border) in &tiles {
        match tile_owner {
            Some(owner) => {
                if let Some(player_entity) = owner.player_entity
                    && let Ok(player) = players.get(player_entity)
                {
                    let desired = OwnershipBorder {
                        color_index: player.color_index,
                    };
                    if existing_border
                        .map(|existing| existing.color_index != desired.color_index)
                        .unwrap_or(true)
                    {
                        commands.entity(entity).insert(desired);
                    }
                    continue;
                }
                if existing_border.is_some() {
                    commands.entity(entity).remove::<OwnershipBorder>();
                }
            }
            None => {
                if existing_border.is_some() {
                    commands.entity(entity).remove::<OwnershipBorder>();
                }
            }
        }
    }
}
