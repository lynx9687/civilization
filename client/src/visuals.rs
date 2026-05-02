use bevy::prelude::*;
use shared::{
    cities::City,
    components::*,
    hex::{HexPosition, hex_to_pixel},
    units::*,
};

use crate::HEX_SIZE;

const SQUARE_SIZE: f32 = 20.0;
const CITY_SIZE: f32 = 28.0;
const UNIT_MOVE_SPEED: f32 = 300.0;

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

//adds mesh for spawned units
pub fn spawn_unit_visuals(
    units: Query<(Entity, &ColorIndex, &HexPosition), Added<Unit>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, color_index, pos) in &units {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let color = player_color(color_index.0);
        println!("Adding unit: {entity}, at pixel {pixel}");
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Circle::new(SQUARE_SIZE))),
            MeshMaterial2d(materials.add(color)),
            Transform::from_xyz(pixel.x, pixel.y, 1.0),
        ));
    }
}

pub fn spawn_city_visuals(
    cities: Query<(Entity, &ColorIndex, &HexPosition), Added<City>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, color_index, pos) in &cities {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let color = player_color(color_index.0);
        println!("Adding city: {entity}, at pixel {pixel}");
        commands.entity(entity).insert((
            Mesh2d(meshes.add(RegularPolygon::new(CITY_SIZE, 4))),
            MeshMaterial2d(materials.add(color)),
            Transform::from_xyz(pixel.x, pixel.y, 2.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
        ));
    }
}

pub fn update_unit_positions(
    time: Res<Time>,
    mut units: Query<(&HexPosition, &mut Transform), With<Unit>>,
) {
    for (pos, mut transform) in &mut units {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let current = transform.translation.truncate();
        let delta = pixel - current;
        let distance = delta.length();
        let max_step = UNIT_MOVE_SPEED * time.delta_secs();

        if distance <= max_step {
            transform.translation.x = pixel.x;
            transform.translation.y = pixel.y;
        } else {
            let step = delta / distance * max_step;
            transform.translation.x += step.x;
            transform.translation.y += step.y;
        }
    }
}
