use bevy::prelude::*;
use shared::{
    components::*,
    hex::{HexPosition, hex_to_pixel},
};

use crate::HEX_SIZE;

const SQUARE_SIZE: f32 = 20.0;

/// Handles to shared hex materials for highlighting.
#[derive(Resource)]
pub struct HexMaterials {
    pub default: Handle<ColorMaterial>,
    pub hovered: Handle<ColorMaterial>,
    pub valid_move: Handle<ColorMaterial>,
}

pub fn setup_camera(mut commands: Commands, mut materials: ResMut<Assets<ColorMaterial>>) {
    commands.spawn(Camera2d);

    let hex_materials = HexMaterials {
        default: materials.add(Color::srgb(0.15, 0.15, 0.2)),
        hovered: materials.add(Color::srgb(0.3, 0.3, 0.4)),
        valid_move: materials.add(Color::srgb(0.2, 0.4, 0.2)),
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

pub fn spawn_player_visuals(
    players: Query<(Entity, &Player, &HexPosition), Added<Player>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, player, pos) in &players {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let color = player_color(player.color_index);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Rectangle::new(SQUARE_SIZE, SQUARE_SIZE))),
            MeshMaterial2d(materials.add(color)),
            Transform::from_xyz(pixel.x, pixel.y, 1.0),
        ));
    }
}

#[allow(clippy::type_complexity)]
pub fn update_player_positions(
    mut players: Query<(&HexPosition, &mut Transform), (With<Player>, Changed<HexPosition>)>,
) {
    for (pos, mut transform) in &mut players {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        transform.translation.x = pixel.x;
        transform.translation.y = pixel.y;
    }
}
