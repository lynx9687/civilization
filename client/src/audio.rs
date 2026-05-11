use bevy::{audio::Volume, prelude::*};

const BACKGROUND_MUSIC_PATH: &str = "music/main_theme.ogg";
const BACKGROUND_MUSIC_VOLUME: f32 = 0.35;

/// Marker for the looping background soundtrack entity.
#[derive(Component)]
pub struct BackgroundMusic;

/// Starts the client's looping background soundtrack.
pub fn play_background_music(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((
        BackgroundMusic,
        AudioPlayer::new(asset_server.load(BACKGROUND_MUSIC_PATH)),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(BACKGROUND_MUSIC_VOLUME)),
    ));
}
