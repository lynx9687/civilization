use bevy::prelude::*;
use shared::{
    components::HexTile,
    hex::{HexPosition, hex_to_pixel},
};

use crate::HEX_SIZE;

const HEX_TINT_STRENGTH: f32 = 2.0;

/// Handles to shared hex materials for highlighting.
#[derive(Resource)]
pub struct HexMaterials {
    pub default: Handle<ColorMaterial>,
    pub hovered: Handle<ColorMaterial>,
    pub valid_move: Handle<ColorMaterial>,
    pub claimed: Vec<Handle<ColorMaterial>>,
    pub valid_attack: Handle<ColorMaterial>,
}

pub fn setup_hex_materials(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let default_texture = asset_server.load("textures/tiles/grass.png");
    let hex_materials = HexMaterials {
        default: materials.add(hex_material(default_texture.clone(), Color::WHITE)),
        hovered: materials.add(tinted_hex_material(default_texture.clone(), 0.3, 0.3, 0.4)),
        valid_move: materials.add(tinted_hex_material(default_texture.clone(), 0.2, 0.4, 0.2)),
        claimed: vec![
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.35,
                0.12,
                0.12,
            )),
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.10,
                0.16,
                0.36,
            )),
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.10,
                0.30,
                0.12,
            )),
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.36,
                0.32,
                0.10,
            )),
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.34,
                0.12,
                0.34,
            )),
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.10,
                0.32,
                0.34,
            )),
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.36,
                0.22,
                0.10,
            )),
            materials.add(tinted_hex_material(
                default_texture.clone(),
                0.22,
                0.12,
                0.36,
            )),
        ],
        valid_attack: materials.add(tinted_hex_material(
            default_texture.clone(),
            0.5,
            0.15,
            0.15,
        )),
    };
    commands.insert_resource(hex_materials);
}

pub fn spawn_hex_visuals(
    tiles: Query<(Entity, &HexPosition), Added<HexTile>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    hex_materials: Res<HexMaterials>,
) {
    for (entity, pos) in &tiles {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(RegularPolygon::new(HEX_SIZE * 0.95, 6))),
            MeshMaterial2d(hex_materials.default.clone()),
            Transform::from_xyz(pixel.x, pixel.y, 0.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_6)),
        ));
    }
}

fn tinted_hex_material(texture: Handle<Image>, red: f32, green: f32, blue: f32) -> ColorMaterial {
    let soften = |channel| 0.75 + channel * HEX_TINT_STRENGTH;
    hex_material(
        texture,
        Color::srgb(soften(red), soften(green), soften(blue)),
    )
}

fn hex_material(texture: Handle<Image>, color: Color) -> ColorMaterial {
    ColorMaterial {
        color,
        texture: Some(texture),
        ..default()
    }
}
