use gpui::WindowAppearance;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    #[serde(other)]
    System,
}

impl Theme {
    pub fn palette(self, appearance: WindowAppearance) -> Palette {
        match self {
            Self::Light => Palette::LIGHT,
            Self::Dark => Palette::DARK,
            Self::System => match appearance {
                WindowAppearance::Light | WindowAppearance::VibrantLight => Palette::LIGHT,
                WindowAppearance::Dark | WindowAppearance::VibrantDark => Palette::DARK,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: u32,
    pub panel: u32,
    pub line: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub thumb: u32,
    pub hover: u32,
    pub tool_hover: u32,
    pub menu_hover: u32,
    pub selected_menu_hover: u32,
    pub menu: u32,
    pub menu_border: u32,
    pub active: u32,
    pub subtle: u32,
    pub success: u32,
    pub warning: u32,
    pub error: u32,
    pub danger_hover: u32,
    pub accent_hover: u32,
    pub accent_border: u32,
    pub logo: u32,
    pub overlay: u32,
    pub wave_axis: u32,
    pub wave_idle: u32,
    pub playhead: u32,
    pub playhead_glow: u32,
}

impl Palette {
    pub const DARK: Self = Self {
        bg: 0x141c27,
        panel: 0x1b2533,
        line: 0x2b384a,
        text: 0xe9eff8,
        muted: 0x8c9cb1,
        accent: 0x388cff,
        on_accent: 0xe9eff8,
        thumb: 0xe9eff8,
        hover: 0x26364b,
        tool_hover: 0x263345,
        menu_hover: 0x36577d,
        selected_menu_hover: 0x36577d,
        menu: 0x202c3d,
        menu_border: 0x3b4d65,
        active: 0x203d60,
        subtle: 0x65758c,
        success: 0x65d6ac,
        warning: 0xffb580,
        error: 0xffa9a9,
        danger_hover: 0x663142,
        accent_hover: 0x55a0ff,
        accent_border: 0x62a7ff,
        logo: 0x49caff,
        overlay: 0x080d16df,
        wave_axis: 0x26374d,
        wave_idle: 0x526d8e,
        playhead: 0x9fceff,
        playhead_glow: 0x6db9ff14,
    };

    pub const LIGHT: Self = Self {
        bg: 0xf4f7fb,
        panel: 0xffffff,
        line: 0xcbd5e3,
        text: 0x182638,
        muted: 0x52657a,
        accent: 0x2162cc,
        on_accent: 0xffffff,
        thumb: 0xffffff,
        hover: 0xe4ecf7,
        tool_hover: 0xe4ecf7,
        menu_hover: 0xe4ecf7,
        selected_menu_hover: 0x174fae,
        menu: 0xffffff,
        menu_border: 0xb9c8dc,
        active: 0xdce9ff,
        subtle: 0x52657a,
        success: 0x16734f,
        warning: 0x995111,
        error: 0xb42332,
        danger_hover: 0xfce0e4,
        accent_hover: 0x174fae,
        accent_border: 0x2162cc,
        logo: 0x2162cc,
        overlay: 0x63738a40,
        wave_axis: 0xcbd5e3,
        wave_idle: 0xa7b9d0,
        playhead: 0x123f82,
        playhead_glow: 0x2162cc18,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_follows_appearance_and_explicit_choices_override_it() {
        for appearance in [WindowAppearance::Light, WindowAppearance::VibrantLight] {
            assert_eq!(Theme::System.palette(appearance), Palette::LIGHT);
            assert_eq!(Theme::Dark.palette(appearance), Palette::DARK);
        }
        for appearance in [WindowAppearance::Dark, WindowAppearance::VibrantDark] {
            assert_eq!(Theme::System.palette(appearance), Palette::DARK);
            assert_eq!(Theme::Light.palette(appearance), Palette::LIGHT);
        }
    }
}
