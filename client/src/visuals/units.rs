use bevy::prelude::*;
use shared::{
    components::player_color,
    hex::{HexPosition, hex_to_pixel},
    unit_definition::UnitRegistry,
    units::{ColorIndex, MoveTo, Unit},
};

use crate::HEX_SIZE;

const UNIT_SPRITE_SIZE: f32 = 50.0;
const UNIT_MOVE_SPEED: f32 = 300.0;
const UNIT_ROTATION_SPEED: f32 = std::f32::consts::PI * 1.0;

pub fn spawn_unit_visuals(
    units: Query<(Entity, &Unit, &ColorIndex, &HexPosition), Added<Unit>>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    registry: Res<UnitRegistry>,
) {
    for (entity, unit, color_index, pos) in &units {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let color = player_color(color_index.0);
        let name = registry.name_of(unit.type_id).unwrap_or("");
        let Some((texture, team_mask_texture)) = unit_texture_paths(name) else {
            eprintln!(
                "Unknown unit type id {:?}, skipping visual spawn",
                unit.type_id
            );
            continue;
        };

        println!("Adding unit: {entity} (type {name}) at pixel {pixel}");
        commands
            .entity(entity)
            .insert((
                unit_sprite(asset_server.load(texture), Color::WHITE),
                Transform::from_xyz(pixel.x, pixel.y, 2.0),
            ))
            .with_children(|parent| {
                parent.spawn((
                    unit_sprite(asset_server.load(team_mask_texture), color),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
    }
}

pub fn update_unit_positions(
    time: Res<Time>,
    mut units: Query<(&HexPosition, Option<&MoveTo>, &mut Transform), With<Unit>>,
) {
    for (pos, move_to, mut transform) in &mut units {
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

        if let Some(move_to) = move_to {
            rotate_toward_move_target(&time, pos, move_to, &mut transform);
        }
    }
}

fn unit_texture_paths(name: &str) -> Option<(String, String)> {
    if name.is_empty() {
        return None;
    }

    Some((
        format!("textures/units/{name}.png"),
        format!("textures/units/{name}_team_mask.png"),
    ))
}

fn unit_sprite(image: Handle<Image>, color: Color) -> Sprite {
    Sprite {
        image,
        color,
        custom_size: Some(Vec2::splat(UNIT_SPRITE_SIZE)),
        ..default()
    }
}

fn rotate_toward_move_target(
    time: &Time,
    pos: &HexPosition,
    move_to: &MoveTo,
    transform: &mut Transform,
) {
    let from = hex_to_pixel(pos, HEX_SIZE);
    let to = hex_to_pixel(&move_to.pos, HEX_SIZE);
    let direction = to - from;

    if direction.length_squared() == 0.0 {
        return;
    }

    let desired_angle = -direction.x.atan2(direction.y);
    let (_, _, current_angle) = transform.rotation.to_euler(EulerRot::XYZ);
    let angle_delta = shortest_angle_delta(current_angle, desired_angle);
    let max_step = UNIT_ROTATION_SPEED * time.delta_secs();

    let next_angle = if angle_delta.abs() <= max_step {
        desired_angle
    } else {
        current_angle + angle_delta.signum() * max_step
    };

    transform.rotation = Quat::from_rotation_z(next_angle);
}

fn shortest_angle_delta(from: f32, to: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
