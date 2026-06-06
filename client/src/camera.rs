use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

const CAMERA_MOVE_SPEED: f32 = 500.0;
const CAMERA_ZOOM_FACTOR: f32 = 1.15;
const CAMERA_MIN_ZOOM: f32 = 0.35;
const CAMERA_MAX_ZOOM: f32 = 4.0;
const CAMERA_ZOOM_SMOOTHING: f32 = 12.0;
const PIXELS_PER_SCROLL_LINE: f32 = 100.0;

/// Tracks the target orthographic camera scale for smoothed scroll-wheel zoom.
#[derive(Resource)]
pub struct CameraZoom {
    target_scale: f32,
}

impl Default for CameraZoom {
    fn default() -> Self {
        Self { target_scale: 1.0 }
    }
}

pub fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

pub fn move_camera_with_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut cameras: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut transform) = cameras.single_mut() else {
        return;
    };

    let mut direction = Vec2::ZERO;

    if keys.pressed(KeyCode::KeyW) {
        direction.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        direction.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }

    if direction == Vec2::ZERO {
        return;
    }

    let movement = direction.normalize() * CAMERA_MOVE_SPEED * time.delta_secs();
    transform.translation.x += movement.x;
    transform.translation.y += movement.y;
}

pub fn zoom_camera_with_scroll(
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut zoom: ResMut<CameraZoom>,
    mut cameras: Query<&mut Projection, With<Camera2d>>,
) {
    let Ok(mut projection) = cameras.single_mut() else {
        return;
    };
    let Projection::Orthographic(projection) = projection.as_mut() else {
        return;
    };

    if scroll.delta.y != 0.0 {
        let scroll_lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y, //used in native client
            MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_SCROLL_LINE, //used in web client
        };
        let zoom_multiplier = CAMERA_ZOOM_FACTOR.powf(-scroll_lines);

        zoom.target_scale =
            (zoom.target_scale * zoom_multiplier).clamp(CAMERA_MIN_ZOOM, CAMERA_MAX_ZOOM);
    }

    let smoothing = 1.0 - (-CAMERA_ZOOM_SMOOTHING * time.delta_secs()).exp();
    projection.scale += (zoom.target_scale - projection.scale) * smoothing;

    if (projection.scale - zoom.target_scale).abs() < 0.001 {
        projection.scale = zoom.target_scale;
    }
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraZoom>()
            .add_systems(Startup, setup_camera)
            .add_systems(Update, (move_camera_with_keyboard, zoom_camera_with_scroll));
    }
}
