use bevy::prelude::*;
use bevy::sprite::Anchor;
use shared::{
    cities::{City, CityStats},
    components::player_color,
    hex::{HexPosition, hex_to_pixel},
    units::{ColorIndex, Health},
};

use crate::HEX_SIZE;

const CITY_SPRITE_SIZE: f32 = 58.0;
const CITY_HEALTH_BAR_WIDTH: f32 = 42.0;
const CITY_HEALTH_BAR_HEIGHT: f32 = 10.0;
const CITY_HEALTH_BAR_OFFSET: Vec3 = Vec3::new(0.0, 48.0, 0.8);
const CITY_HEALTH_BAR_FONT_SIZE: f32 = 10.0;
const CITY_HEALTH_BAR_FILL_COLOR: Color = Color::srgb(0.02, 0.12, 0.65);

#[derive(Clone, Copy)]
enum CityVisualTier {
    Small,
    Medium,
    Large,
}

#[derive(Component)]
pub(crate) struct CityVisual {
    team_mask: Entity,
}

#[derive(Component)]
pub(crate) struct CityHealthBar {
    fill: Entity,
    text: Entity,
}

#[derive(Component)]
pub(crate) struct CityTeamMask;

#[derive(Component)]
pub(crate) struct CityHealthBarFill;

#[derive(Component)]
pub(crate) struct CityHealthBarText;

type CityHealthBarFillFilter = (With<CityHealthBarFill>, Without<City>);

pub fn spawn_city_visuals(
    cities: Query<(Entity, &HexPosition, &CityStats, &ColorIndex), Added<City>>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    for (entity, pos, stats, color_index) in &cities {
        let pixel = hex_to_pixel(pos, HEX_SIZE);
        let tier = CityVisualTier::for_border_range(stats.border_range);
        let color = player_color(color_index.0);
        let (texture, team_mask_texture) = tier.texture_paths();

        println!("Adding city: {entity}, at pixel {pixel}");
        let mut team_mask = None;
        let mut health_bar_fill = None;
        let mut health_bar_text = None;
        commands
            .entity(entity)
            .insert((
                city_sprite(asset_server.load(texture), Color::WHITE),
                Transform::from_xyz(pixel.x, pixel.y, 1.5),
            ))
            .with_children(|parent| {
                team_mask = Some(
                    parent
                        .spawn((
                            CityTeamMask,
                            city_sprite(asset_server.load(team_mask_texture), color),
                            Transform::from_xyz(0.0, 0.0, 0.1),
                        ))
                        .id(),
                );

                let mut health_bar = parent.spawn((
                    Sprite {
                        color: Color::srgba(0.05, 0.05, 0.05, 0.9),
                        custom_size: Some(Vec2::new(CITY_HEALTH_BAR_WIDTH, CITY_HEALTH_BAR_HEIGHT)),
                        ..default()
                    },
                    Transform::from_translation(CITY_HEALTH_BAR_OFFSET),
                ));

                health_bar.with_children(|bar| {
                    health_bar_fill = Some(
                        bar.spawn((
                            CityHealthBarFill,
                            Sprite {
                                color: CITY_HEALTH_BAR_FILL_COLOR,
                                custom_size: Some(Vec2::new(
                                    CITY_HEALTH_BAR_WIDTH,
                                    CITY_HEALTH_BAR_HEIGHT,
                                )),
                                ..default()
                            },
                            Transform::from_xyz(0.0, 0.0, 0.1),
                        ))
                        .id(),
                    );

                    health_bar_text = Some(
                        bar.spawn((
                            CityHealthBarText,
                            Text2d::new(""),
                            TextFont {
                                font_size: CITY_HEALTH_BAR_FONT_SIZE,
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
            CityVisual {
                team_mask: team_mask.expect("city team mask should be spawned"),
            },
            CityHealthBar {
                fill: health_bar_fill.expect("city health bar fill should be spawned"),
                text: health_bar_text.expect("city health bar text should be spawned"),
            },
        ));
    }
}

#[allow(clippy::type_complexity)]
pub fn update_city_visuals(
    cities: Query<
        (&CityStats, &ColorIndex, &mut Sprite, &CityVisual),
        (With<City>, Or<(Changed<CityStats>, Changed<ColorIndex>)>),
    >,
    mut masks: Query<&mut Sprite, (With<CityTeamMask>, Without<City>)>,
    asset_server: Res<AssetServer>,
) {
    for (stats, color_index, mut sprite, visual) in cities {
        let tier = CityVisualTier::for_border_range(stats.border_range);
        let (texture, team_mask_texture) = tier.texture_paths();
        let color = player_color(color_index.0);

        sprite.image = asset_server.load(texture);
        if let Ok(mut mask_sprite) = masks.get_mut(visual.team_mask) {
            mask_sprite.image = asset_server.load(team_mask_texture);
            mask_sprite.color = color;
        }
    }
}

pub fn update_city_health_bars(
    cities: Query<(&Health, &CityHealthBar), With<City>>,
    mut bar_fills: Query<(&mut Sprite, &mut Transform), CityHealthBarFillFilter>,
    mut bar_texts: Query<&mut Text2d, With<CityHealthBarText>>,
) {
    for (health, health_bar) in &cities {
        let Ok((mut fill_sprite, mut fill_transform)) = bar_fills.get_mut(health_bar.fill) else {
            continue;
        };

        let health_fraction = if health.max == 0 {
            0.0
        } else {
            health.current as f32 / health.max as f32
        }
        .clamp(0.0, 1.0);
        let fill_width = CITY_HEALTH_BAR_WIDTH * health_fraction;

        fill_sprite.custom_size = Some(Vec2::new(fill_width, CITY_HEALTH_BAR_HEIGHT));
        fill_transform.translation.x = (fill_width - CITY_HEALTH_BAR_WIDTH) * 0.5;

        if let Ok(mut text) = bar_texts.get_mut(health_bar.text) {
            text.0 = format!("{}/{}", health.current, health.max);
        }
    }
}

impl CityVisualTier {
    fn for_border_range(border_range: i32) -> Self {
        match border_range {
            i32::MIN..=1 => Self::Small,
            2 => Self::Medium,
            3..=i32::MAX => Self::Large,
        }
    }

    fn texture_paths(self) -> (&'static str, &'static str) {
        match self {
            Self::Small => (
                "textures/buildings/castle_small.png",
                "textures/buildings/castle_small_team_mask.png",
            ),
            Self::Medium => (
                "textures/buildings/castle_medium.png",
                "textures/buildings/castle_medium_team_mask.png",
            ),
            Self::Large => (
                "textures/buildings/castle_large.png",
                "textures/buildings/castle_large_team_mask.png",
            ),
        }
    }
}

fn city_sprite(image: Handle<Image>, color: Color) -> Sprite {
    Sprite {
        image,
        color,
        custom_size: Some(Vec2::splat(CITY_SPRITE_SIZE)),
        ..default()
    }
}
