use bevy::prelude::*;
use bevy::sprite::Anchor;
use shared::{
    components::player_color,
    hex::{HexPosition, hex_to_pixel},
    unit_definition::UnitRegistry,
    units::{AttackTarget, ColorIndex, Health, MoveTo, Unit},
};

use crate::HEX_SIZE;

const UNIT_SPRITE_SIZE: f32 = 50.0;
const UNIT_MOVE_SPEED: f32 = 300.0;
const UNIT_ROTATION_SPEED: f32 = std::f32::consts::PI * 1.0;
const HEALTH_BAR_WIDTH: f32 = 42.0;
const HEALTH_BAR_HEIGHT: f32 = 10.0;
const HEALTH_BAR_OFFSET: Vec3 = Vec3::new(0.0, 32.0, 0.4);
const HEALTH_BAR_FONT_SIZE: f32 = 10.0;

#[derive(Component)]
#[require(Transform, Visibility)]
pub(crate) struct UnitVisual {
    sprite_root: Entity,
    team_mask: Entity,
}

#[derive(Component)]
pub(crate) struct UnitSpriteRoot;

/// Marks the team-colour mask sprite that is a child of `UnitSpriteRoot`.
/// Stored separately so `update_unit_colors` can reach it without walking the hierarchy.
#[derive(Component)]
pub(crate) struct UnitTeamMask;

#[derive(Component)]
pub(crate) struct UnitHealthBar {
    fill: Entity,
    text: Entity,
}

#[derive(Component)]
pub(crate) struct HealthBar;

#[derive(Component)]
pub(crate) struct HealthBarFill;

#[derive(Component)]
pub(crate) struct HealthBarText;

type UnitSpriteRootFilter = (With<UnitSpriteRoot>, Without<Unit>);
type HealthBarFillFilter = (With<HealthBarFill>, Without<Unit>);

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
        let mut sprite_root = None;
        let mut team_mask = None;
        let mut health_bar_fill = None;
        let mut health_bar_text = None;

        commands
            .entity(entity)
            .insert((
                Transform::from_xyz(pixel.x, pixel.y, 2.0),
                Visibility::Inherited,
            ))
            .with_children(|parent| {
                let mut unit_sprite_root = parent.spawn((
                    UnitSpriteRoot,
                    unit_sprite(asset_server.load(texture), Color::WHITE),
                    Transform::default(),
                ));
                sprite_root = Some(unit_sprite_root.id());

                unit_sprite_root.with_children(|sprite| {
                    team_mask = Some(
                        sprite
                            .spawn((
                                UnitTeamMask,
                                unit_sprite(asset_server.load(team_mask_texture), color),
                                Transform::from_xyz(0.0, 0.0, 0.1),
                            ))
                            .id(),
                    );
                });

                let mut health_bar = parent.spawn((
                    HealthBar,
                    Sprite {
                        color: Color::srgba(0.05, 0.05, 0.05, 0.9),
                        custom_size: Some(Vec2::new(HEALTH_BAR_WIDTH, HEALTH_BAR_HEIGHT)),
                        ..default()
                    },
                    Transform::from_translation(HEALTH_BAR_OFFSET),
                ));

                health_bar.with_children(|bar| {
                    health_bar_fill = Some(
                        bar.spawn((
                            HealthBarFill,
                            Sprite {
                                color: Color::srgb(0.85, 0.25, 0.25),
                                custom_size: Some(Vec2::new(HEALTH_BAR_WIDTH, HEALTH_BAR_HEIGHT)),
                                ..default()
                            },
                            Transform::from_xyz(0.0, 0.0, 0.1),
                        ))
                        .id(),
                    );

                    health_bar_text = Some(
                        bar.spawn((
                            HealthBarText,
                            Text2d::new(""),
                            TextFont {
                                font_size: HEALTH_BAR_FONT_SIZE,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            Anchor::CENTER,
                            Transform::from_xyz(0.0, 0.0, 0.2),
                        ))
                        .id(),
                    );
                });
            });

        commands.entity(entity).insert((
            UnitVisual {
                sprite_root: sprite_root.expect("unit sprite root should be spawned"),
                team_mask: team_mask.expect("unit team mask should be spawned"),
            },
            UnitHealthBar {
                fill: health_bar_fill.expect("health bar fill should be spawned"),
                text: health_bar_text.expect("health bar text should be spawned"),
            },
        ));
    }
}

#[allow(clippy::type_complexity)]
pub fn update_unit_positions(
    time: Res<Time>,
    mut units: Query<
        (
            &HexPosition,
            Option<&MoveTo>,
            Option<&AttackTarget>,
            &mut Transform,
            &UnitVisual,
        ),
        With<Unit>,
    >,
    mut sprite_roots: Query<&mut Transform, UnitSpriteRootFilter>,
) {
    for (pos, move_to, attack_target, mut transform, visual) in &mut units {
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

        let facing_target = move_to
            .map(|move_to| move_to.pos)
            .or_else(|| attack_target.map(|attack_target| attack_target.pos));

        if let Some(target_pos) = facing_target
            && let Ok(mut sprite_transform) = sprite_roots.get_mut(visual.sprite_root)
        {
            rotate_toward_target(&time, pos, &target_pos, &mut sprite_transform);
        }
    }
}

pub fn update_unit_health_bars(
    units: Query<(&Health, &UnitHealthBar), With<Unit>>,
    mut bar_fills: Query<(&mut Sprite, &mut Transform), HealthBarFillFilter>,
    mut bar_texts: Query<&mut Text2d, With<HealthBarText>>,
) {
    for (health, health_bar) in &units {
        let Ok((mut fill_sprite, mut fill_transform)) = bar_fills.get_mut(health_bar.fill) else {
            continue;
        };

        let health_fraction = if health.max == 0 {
            0.0
        } else {
            health.current as f32 / health.max as f32
        }
        .clamp(0.0, 1.0);
        let fill_width = HEALTH_BAR_WIDTH * health_fraction;

        fill_sprite.custom_size = Some(Vec2::new(fill_width, HEALTH_BAR_HEIGHT));
        fill_transform.translation.x = (fill_width - HEALTH_BAR_WIDTH) * 0.5;

        if let Ok(mut text) = bar_texts.get_mut(health_bar.text) {
            text.0 = format!("{}/{}", health.current, health.max);
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

/// Repaints team-mask sprites whenever the server replicates a new `ColorIndex`
/// (e.g. after lobby slot reindexing).  `spawn_unit_visuals` only runs once for
/// `Added<Unit>`, so without this system colors would be frozen at join time.
#[allow(clippy::type_complexity)]
pub fn update_unit_colors(
    units: Query<(&ColorIndex, &UnitVisual), (With<Unit>, Changed<ColorIndex>)>,
    mut masks: Query<&mut Sprite, With<UnitTeamMask>>,
) {
    for (color_index, visual) in &units {
        if let Ok(mut sprite) = masks.get_mut(visual.team_mask) {
            sprite.color = player_color(color_index.0);
        }
    }
}

fn rotate_toward_target(
    time: &Time,
    pos: &HexPosition,
    target: &HexPosition,
    transform: &mut Transform,
) {
    let from = hex_to_pixel(pos, HEX_SIZE);
    let to = hex_to_pixel(target, HEX_SIZE);
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
