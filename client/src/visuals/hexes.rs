use bevy::prelude::*;
use shared::{
    components::{DefeatedPlayer, HexTile, OwnershipBorder, Player, PLAYER_COLORS},
    hex::{HexPosition, hex_to_pixel},
    terrain::Terrain,
    tiles::TileOwner,
};

use crate::HEX_SIZE;

const HEX_TINT_STRENGTH: f32 = 1.0;

/// Handles to shared hex materials for highlighting.
#[derive(Resource)]
pub struct HexMaterials {
    pub default: Handle<ColorMaterial>,
    /// Per-terrain hover materials (brightened versions), indexed by `terrain as usize`.
    pub hovered: Vec<Handle<ColorMaterial>>,
    pub valid_move: Handle<ColorMaterial>,
    pub valid_attack: Handle<ColorMaterial>,
    /// Base material per terrain, indexed by `terrain as usize` (see `Terrain::ALL`).
    /// Each terrain gets a distinct, readable base (see `terrain_material`).
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

pub fn setup_hex_materials(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let default_texture = asset_server.load("textures/tiles/grass.png");
    let hex_materials = HexMaterials {
        default: materials.add(hex_material(default_texture.clone(), Color::WHITE)),
        // Per-terrain brightened hover materials, indexed by `terrain as usize`.
        // Iterating `Terrain::ALL` keeps index == discriminant reorder-safe.
        hovered: Terrain::ALL
            .iter()
            .map(|t| materials.add(terrain_hovered_material(default_texture.clone(), *t)))
            .collect(),
        valid_move: materials.add(tinted_hex_material(
            default_texture.clone(),
            Color::srgb(0.4, 0.8, 0.4),
        )),
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

/// Brightened hover material for each terrain type, maintaining terrain identity
/// while appearing brightened. Values chosen to increase luminosity while preserving hue.
fn terrain_hovered_material(texture: Handle<Image>, terrain: Terrain) -> ColorMaterial {
    match terrain {
        // Brighten the grass texture while keeping it readable as grassland.
        Terrain::Grassland => hex_material(texture, Color::srgb(1.5, 1.3, 1.2)),
        // Brighten the olive/tan hill while preserving the hue.
        Terrain::Hill => hex_material(texture, Color::srgb(1.6, 1.2, 1.3)),
        // Brighten the dark green forest while keeping it recognizable.
        Terrain::Forest => hex_material(texture, Color::srgb(0.6, 0.9, 0.5)),
        // Brightened gray flat fill for mountain.
        Terrain::Mountain => flat_material(Color::srgb(0.7, 0.7, 0.7)),
        // Brightened blue flat fill for water.
        Terrain::Water => flat_material(Color::srgb(0.4, 0.65, 1.0)),
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

#[derive(Component)]
pub(crate) struct OwnershipBorderVisual;

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
                Mesh2d(meshes.add(RegularPolygon::new(border_size, 6))),
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

pub fn sync_ownership_borders(
    mut commands: Commands,
    tiles: Query<(Entity, Option<&TileOwner>, Option<&OwnershipBorder>), With<HexTile>>,
    players: Query<&Player, Without<DefeatedPlayer>>,
) {
    for (entity, tile_owner, existing_border) in &tiles {
        match tile_owner {
            Some(owner) => {
                if let Some(player_entity) = owner.player_entity {
                    if let Ok(player) = players.get(player_entity) {
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
