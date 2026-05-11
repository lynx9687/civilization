use bevy::prelude::*;
use shared::{
    cities::{City, CityStats},
    components::player_color,
    hex::{HexPosition, hex_to_pixel},
    units::ColorIndex,
};

use crate::HEX_SIZE;

const CITY_SPRITE_SIZE: f32 = 58.0;

#[derive(Clone, Copy)]
enum CityVisualTier {
    Small,
    Medium,
    Large,
}

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
        commands
            .entity(entity)
            .insert((
                city_sprite(asset_server.load(texture), Color::WHITE),
                Transform::from_xyz(pixel.x, pixel.y, 1.5),
            ))
            .with_children(|parent| {
                parent.spawn((
                    city_sprite(asset_server.load(team_mask_texture), color),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
    }
}

#[allow(clippy::type_complexity)]
pub fn update_city_visuals(
    cities: Query<
        (&CityStats, &ColorIndex, &mut Sprite, &Children),
        (With<City>, Or<(Changed<CityStats>, Changed<ColorIndex>)>),
    >,
    mut sprites: Query<&mut Sprite, Without<City>>,
    asset_server: Res<AssetServer>,
) {
    for (stats, color_index, mut sprite, children) in cities {
        let tier = CityVisualTier::for_border_range(stats.border_range);
        let (texture, team_mask_texture) = tier.texture_paths();
        let color = player_color(color_index.0);

        sprite.image = asset_server.load(texture);
        for child in children {
            let Ok(mut mask_sprite) = sprites.get_mut(*child) else {
                continue;
            };
            mask_sprite.image = asset_server.load(team_mask_texture);
            mask_sprite.color = color;
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
