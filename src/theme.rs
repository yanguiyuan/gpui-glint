/// A cohesive set of neutral surface colors used by the Glint UI playground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub canvas: u32,
    pub workspace: u32,
    pub panel: u32,
    pub surface: u32,
    pub surface_subtle: u32,
    pub preview_start: u32,
    pub preview_end: u32,
    pub border: u32,
    pub text: u32,
    pub text_muted: u32,
    pub accent: u32,
    pub accent_tint: u32,
    pub green_tint: u32,
}

/// Six light surface palettes and five dark appearances.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreset {
    FogWhite,
    ShallowBay,
    #[default]
    Sage,
    Linen,
    SilverGray,
    Moonstone,
    DarkFogWhite,
    DarkShallowBay,
    DarkLinen,
    DarkSilverGray,
    DarkObsidian,
}

impl ThemePreset {
    pub const ALL: [Self; 11] = [
        Self::FogWhite,
        Self::ShallowBay,
        Self::Sage,
        Self::Linen,
        Self::SilverGray,
        Self::Moonstone,
        Self::DarkFogWhite,
        Self::DarkShallowBay,
        Self::DarkLinen,
        Self::DarkSilverGray,
        Self::DarkObsidian,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::FogWhite => "雾白",
            Self::ShallowBay => "浅湾",
            Self::Sage => "鼠尾草",
            Self::Linen => "亚麻",
            Self::SilverGray => "银灰",
            Self::Moonstone => "月光白",
            Self::DarkFogWhite => "深色雾白",
            Self::DarkShallowBay => "深色浅湾",
            Self::DarkLinen => "深色亚麻",
            Self::DarkSilverGray => "深色银灰",
            Self::DarkObsidian => "曜石黑",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::FogWhite => "最轻的中性底色，适合长时间工作。",
            Self::ShallowBay => "一点冷蓝，清爽而不偏紫。",
            Self::Sage => "低饱和灰绿，柔和、安静。",
            Self::Linen => "温暖的米灰，保留清晰的白色层次。",
            Self::SilverGray => "接近铝合金的淡灰，不带明显色相。",
            Self::Moonstone => "中性亮白玻璃，标准明亮的清晰层次。",
            Self::DarkFogWhite => "中性石墨玻璃，清晰的浅色文字。",
            Self::DarkShallowBay => "深海灰蓝，安静的冷色层次。",
            Self::DarkLinen => "温暖的深米灰，低亮度工作空间。",
            Self::DarkSilverGray => "深银灰表面，克制的中性层次。",
            Self::DarkObsidian => "接近纯黑的中性玻璃，经典暗黑层次。",
        }
    }

    pub const fn palette(self) -> ThemePalette {
        if self.is_dark() {
            let (canvas, preview_start, preview_end) = match self {
                Self::DarkFogWhite => (0x151819, 0x22292c, 0x242d29),
                Self::DarkShallowBay => (0x141b20, 0x22333f, 0x233737),
                Self::DarkLinen => (0x1d1b18, 0x363027, 0x2c322b),
                Self::DarkSilverGray => (0x181a1d, 0x2b2e34, 0x292f31),
                Self::DarkObsidian => (0x141416, 0x202024, 0x1e1e21),
                _ => unreachable!(),
            };
            // Obsidian is the standard near-black set; the other dark presets
            // keep the shared graphite surfaces they have always used.
            let neutral = matches!(self, Self::DarkObsidian);
            let (workspace, panel, surface, surface_subtle, border) = if neutral {
                (0x1b1b1e, 0x212125, 0x212125, 0x303036, 0x38383e)
            } else {
                (0x1b1d23, 0x1e2026, 0x16181e, 0x303238, 0x404248)
            };
            let (text_muted, accent_tint, green_tint) = if neutral {
                (0x9e9ea4, 0x1c3252, 0x2e3d33)
            } else {
                (0xaeaeb2, 0x193654, 0x203e2c)
            };
            return ThemePalette {
                canvas,
                preview_start,
                preview_end,
                workspace,
                panel,
                surface,
                surface_subtle,
                border,
                text: 0xf2f2f7,
                text_muted,
                accent: 0x0a84ff,
                accent_tint,
                green_tint,
            };
        }
        match self {
            Self::FogWhite => ThemePalette {
                canvas: 0xf2f3f2,
                workspace: 0xf8f9f8,
                panel: 0xf6f7f4,
                surface: 0xffffff,
                surface_subtle: 0xe5e9ea,
                preview_start: 0xe5e9ea,
                preview_end: 0xe5ebe8,
                border: 0xe1e5e2,
                text: 0x1d1d1f,
                text_muted: 0x6e6e73,
                accent: 0x007aff,
                accent_tint: 0xdcecff,
                green_tint: 0xe0f2e2,
            },
            Self::ShallowBay => ThemePalette {
                canvas: 0xedf2f5,
                workspace: 0xf3f7f8,
                panel: 0xf0f5f6,
                surface: 0xffffff,
                surface_subtle: 0xd7e3ec,
                preview_start: 0xd7e3ec,
                preview_end: 0xdce9e9,
                border: 0xd3dde1,
                text: 0x1d1d1f,
                text_muted: 0x6e6e73,
                accent: 0x007aff,
                accent_tint: 0xdcecff,
                green_tint: 0xe2f0e7,
            },
            Self::Sage => ThemePalette {
                canvas: 0xeff2ed,
                workspace: 0xf8f9f8,
                panel: 0xf5f7f3,
                surface: 0xffffff,
                surface_subtle: 0xdbe5d9,
                preview_start: 0xdbe5d9,
                preview_end: 0xdce8e1,
                border: 0xe1e5e2,
                text: 0x1d1d1f,
                text_muted: 0x6e6e73,
                accent: 0x007aff,
                accent_tint: 0xdcecff,
                green_tint: 0xe0f2e2,
            },
            Self::Linen => ThemePalette {
                canvas: 0xf4f1eb,
                workspace: 0xf9f6f0,
                panel: 0xf6f2eb,
                surface: 0xfffdf9,
                surface_subtle: 0xe8e1d6,
                preview_start: 0xe8e1d6,
                preview_end: 0xe4e7df,
                border: 0xe1dacf,
                text: 0x1d1d1f,
                text_muted: 0x6e6e73,
                accent: 0x007aff,
                accent_tint: 0xe2edfb,
                green_tint: 0xe4efe5,
            },
            Self::SilverGray => ThemePalette {
                canvas: 0xeff0f2,
                workspace: 0xf6f7f8,
                panel: 0xf2f4f5,
                surface: 0xffffff,
                surface_subtle: 0xdfe2e6,
                preview_start: 0xdfe2e6,
                preview_end: 0xdfe4e5,
                border: 0xd9dde1,
                text: 0x1d1d1f,
                text_muted: 0x6e6e73,
                accent: 0x007aff,
                accent_tint: 0xdcecff,
                green_tint: 0xe1eee5,
            },
            // Moonstone is the standard neutral light set: one quiet gray for
            // the shell, near-white workspace, pure white elevated surfaces.
            Self::Moonstone => ThemePalette {
                canvas: 0xf2f3f3,
                workspace: 0xf8f8f8,
                panel: 0xf2f3f3,
                surface: 0xffffff,
                surface_subtle: 0xe6e7e7,
                preview_start: 0xe6e7e9,
                preview_end: 0xe9eaec,
                border: 0xe3e4e5,
                text: 0x1d1d1f,
                text_muted: 0x6e6e73,
                accent: 0x007aff,
                accent_tint: 0xe7eefe,
                green_tint: 0xe4f1e8,
            },
            _ => unreachable!(),
        }
    }

    pub const fn is_dark(self) -> bool {
        matches!(
            self,
            Self::DarkFogWhite
                | Self::DarkShallowBay
                | Self::DarkLinen
                | Self::DarkSilverGray
                | Self::DarkObsidian
        )
    }

    /// Resolve the original light component tokens to the dark semantic colors.
    /// Light presets retain their existing component appearance.
    pub fn color(self, value: u32) -> u32 {
        if !self.is_dark() {
            return value;
        }
        let p = self.palette();
        match value {
            0x007aff | 0x087bfa => p.accent,
            0x0874e8 | 0x1688ff | 0x2b3c4c | 0x345d8c | 0x69b0ff | 0x6aa9f4 => 0x3d9bff,
            0x16833a | 0x24864c | 0x305343 | 0x34c759 => 0x5be07e,
            0x20242c | 0x262a32 | 0x282b31 | 0x292a35 | 0x292d32 | 0x303036 | 0x30303d
            | 0x303532 | 0x33343c | 0x41454e | 0x48484a | 0x494a55 | 0x4d4e57 | 0x4e5451
            | 0x505451 | 0x505651 | 0x505661 | 0x555b58 | 0x555b59 => p.text,
            0x604832 | 0x92683c | 0xb37716 | 0xff9500 => 0xffb340,
            0x666e6b | 0x6e6e73 | 0x6e7470 | 0x777f7b | 0x787882 | 0x7e8582 | 0x858c89
            | 0x8b928e | 0x8e8e93 | 0x92979a | 0x929895 | 0xaeaeb2 | 0xaeb2b6 => p.text_muted,
            0xa24e32 | 0xd9342b | 0xd95245 | 0xf0443a | 0xff3b30 | 0xff3d36 | 0xff5a50 => 0xff6961,
            0xcfe5ff | 0xd4e8ff | 0xd9e9fc | 0xdcecff | 0xe1efff | 0xe3f0ff | 0xe6f1ff => {
                p.accent_tint
            }
            0xd2d6d4 | 0xd5dae2 | 0xdedee3 | 0xdfe3df | 0xdfe4df | 0xdfe4e0 | 0xe0e3e0
            | 0xe1e5e1 | 0xe1e5e2 | 0xe1e5e9 | 0xe2e3e7 | 0xe2e6e2 | 0xe3e5e8 | 0xe3e7e3
            | 0xe4e6e9 | 0xe5e5ea => p.border,
            0xdde1e8 | 0xe0e0e8 | 0xe7ebef | 0xe8eae8 | 0xe8ebe7 | 0xe8ebe8 | 0xe8ebf0
            | 0xe9ece9 | 0xededef | 0xf0f0f4 | 0xf0f2ee | 0xf0f2f4 | 0xf0f3f0 | 0xf0f4f9
            | 0xf1f4f6 | 0xf2f3f2 | 0xf3f5f8 | 0xf4f4f5 => p.surface_subtle,
            0xe0f2e2 => p.green_tint,
            0xf5f5f7 | 0xf7f8f6 | 0xfafafc | 0xfafcff | 0xfbfbfd | 0xfcfdfc => p.surface,
            0xffc6c1 | 0xffdeda | 0xffe0dc | 0xffe9e7 => 0x492b2d,
            _ => value,
        }
    }

    /// Dark wallpaper colors from the corresponding HTML background preset.
    pub const fn wallpaper_colors(self) -> [u32; 4] {
        match self {
            Self::DarkFogWhite => [0x22292c, 0x292c31, 0x2d2b28, 0x242d29],
            Self::DarkShallowBay => [0x22333f, 0x29343e, 0x253136, 0x233737],
            Self::DarkLinen => [0x363027, 0x38332c, 0x352e29, 0x2c322b],
            Self::DarkSilverGray => [0x2b2e34, 0x303237, 0x333231, 0x292f31],
            Self::DarkObsidian => [0x242429, 0x2b2b30, 0x222226, 0x1e1e23],
            _ => [
                self.palette().preview_start,
                self.palette().canvas,
                self.palette().preview_end,
                self.palette().canvas,
            ],
        }
    }

    /// Ultra-thin glass: the HTML demo surface is rgba(30,32,38,.44).
    pub const fn panel_material(self) -> u32 {
        if self.is_dark() {
            match self {
                Self::DarkObsidian => 0x26262b70,
                _ => 0x1e202670,
            }
        } else {
            (self.palette().panel << 8) | 255
        }
    }

    /// Nearly opaque menu glass carrying the preset's dark hue, so popovers
    /// follow the wallpaper instead of one shared graphite. The HTML demo
    /// surface is rgba(30,32,38,.95) (--g-menu); each dark preset shifts it.
    pub const fn menu_material(self) -> u32 {
        if self.is_dark() {
            match self {
                Self::DarkFogWhite => 0x1e2225f2,
                Self::DarkShallowBay => 0x1b2731f2,
                Self::DarkLinen => 0x28221bf2,
                Self::DarkSilverGray => 0x1f2227f2,
                Self::DarkObsidian => 0x262629f2,
                _ => unreachable!(),
            }
        } else {
            (self.palette().surface << 8) | 255
        }
    }

    pub const fn glass_line(self) -> u32 {
        if self.is_dark() {
            0xffffff1f
        } else {
            (self.palette().surface << 8) | 255
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::FogWhite => "fog-white",
            Self::ShallowBay => "shallow-bay",
            Self::Sage => "sage",
            Self::Linen => "linen",
            Self::SilverGray => "silver-gray",
            Self::Moonstone => "moonstone",
            Self::DarkFogWhite => "dark-fog-white",
            Self::DarkShallowBay => "dark-shallow-bay",
            Self::DarkLinen => "dark-linen",
            Self::DarkSilverGray => "dark-silver-gray",
            Self::DarkObsidian => "dark-obsidian",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let value = ((color >> shift) & 255u32) as f64 / 255.;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    #[test]
    fn theme_ids_round_trip_and_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for theme in ThemePreset::ALL {
            assert!(ids.insert(theme.id()));
            assert_eq!(ThemePreset::from_id(theme.id()), Some(theme));
        }
        assert_eq!(ids.len(), 11);
        assert_eq!(ThemePreset::from_id("dark-sage"), None);
        assert_eq!(ThemePreset::from_id("dark-unknown"), None);
        assert_eq!(ThemePreset::default(), ThemePreset::Sage);
    }

    #[test]
    fn dark_text_has_readable_contrast_on_all_surfaces() {
        for theme in ThemePreset::ALL.into_iter().filter(|theme| theme.is_dark()) {
            let p = theme.palette();
            for background in [p.canvas, p.workspace, p.panel, p.surface, p.surface_subtle] {
                for text in [p.text, p.text_muted] {
                    let contrast = (luminance(text) + 0.05) / (luminance(background) + 0.05);
                    assert!(contrast >= 4.5, "{} contrast: {contrast}", theme.id());
                }
            }
            assert_eq!(theme.color(0xfcfdfc), p.surface);
            assert_eq!(theme.color(0xfafcff), p.surface);
            assert_eq!(theme.color(0xd5dae2), p.border);
            assert_eq!(theme.color(0xdcecff), p.accent_tint);
            assert_eq!(theme.color(0x007aff), p.accent);
            assert_eq!(theme.color(0xffffff), 0xffffff); // white glyphs / switch knobs
        }
    }

    #[test]
    fn dark_menu_glass_is_per_preset_and_nearly_opaque() {
        let mut dark_hues = std::collections::HashSet::new();
        for theme in ThemePreset::ALL.into_iter().filter(|theme| theme.is_dark()) {
            let material = theme.menu_material();
            assert_eq!(material & 0xff, 0xf2, "{} alpha", theme.id());
            dark_hues.insert(material >> 8);
        }
        assert_eq!(dark_hues.len(), 5, "every dark preset needs its own hue");
        for theme in ThemePreset::ALL
            .into_iter()
            .filter(|theme| !theme.is_dark())
        {
            assert_eq!(theme.menu_material() & 0xff, 0xff);
        }
    }

    #[test]
    fn light_component_tokens_keep_their_original_colors() {
        for theme in ThemePreset::ALL
            .into_iter()
            .filter(|theme| !theme.is_dark())
        {
            for token in [0xfcfdfc, 0x007aff, 0xe1e5e2, 0x48484a, 0xdcecff] {
                assert_eq!(theme.color(token), token);
            }
        }
    }
}
