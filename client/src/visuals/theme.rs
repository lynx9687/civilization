use bevy::color::palettes::css::*;
use bevy::prelude::*;

pub struct ColorState {
    pub idle: Color,
    pub hover: Color,
    pub pressed: Color,
    pub waiting: Color,
}

pub struct ButtonTheme {
    pub background: ColorState,
    pub border: ColorState,
}

pub const FINISH_BUTTON: ButtonTheme = ButtonTheme {
    background: ColorState {
        idle: Color::Srgba(DARK_CYAN),
        hover: Color::Srgba(LIGHT_CYAN),
        pressed: Color::Srgba(DARK_SLATE_GRAY),
        waiting: Color::Srgba(DARK_GRAY),
    },
    border: ColorState {
        idle: Color::Srgba(TEAL),
        hover: Color::Srgba(AQUAMARINE),
        pressed: Color::Srgba(SLATE_GRAY),
        waiting: Color::Srgba(GRAY),
    },
};
