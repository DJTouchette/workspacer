//! Native appearance preferences; independent of the hub's shared config.yaml.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    Nord,
    /// Ported from Omarchy's stock themes (`/usr/share/omarchy/themes/<slug>/
    /// colors.toml`); slugs match Omarchy's so the names stay recognizable.
    TokyoNight,
    Catppuccin,
    Gruvbox,
    Everforest,
    CatppuccinLatte,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub base: u32,
    pub surface: u32,
    pub chat: u32,
    pub selected: u32,
    pub border: u32,
    pub text: u32,
    /// Chat body copy; one step below `text` so bold/italic read as emphasis.
    pub prose: u32,
    pub muted: u32,
    pub disabled: u32,
    pub accent: u32,
    pub primary: u32,
    pub primary_hover: u32,
    pub primary_pressed: u32,
    pub on_primary: u32,
    pub warning: u32,
    pub error: u32,
    pub success: u32,
    pub busy: u32,
    pub user: u32,
    pub code_inline: u32,
    pub code_block: u32,
    pub code_header: u32,
    /// Text-selection highlight as 0xRRGGBBAA. Translucent so selected text
    /// and syntax colors stay readable on every surface it crosses.
    pub selection: u32,
    pub shadow: u32,
    pub shadow_opacity: f32,
    pub control_radius: f32,
    pub panel_radius: f32,
    pub composer_radius: f32,
}

impl Appearance {
    pub const ALL: [Self; 8] = [
        Self::Dark,
        Self::Light,
        Self::Nord,
        Self::TokyoNight,
        Self::Catppuccin,
        Self::Gruvbox,
        Self::Everforest,
        Self::CatppuccinLatte,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Nord => "Nord",
            Self::TokyoNight => "Tokyo Night",
            Self::Catppuccin => "Catppuccin Mocha",
            Self::Gruvbox => "Gruvbox",
            Self::Everforest => "Everforest",
            Self::CatppuccinLatte => "Catppuccin Latte",
        }
    }

    /// Light palettes take the component library's light mode, light
    /// switch thumbs and darker generated project colors.
    pub fn is_dark(self) -> bool {
        !matches!(self, Self::Light | Self::CatppuccinLatte)
    }

    /// Native semantic palettes. Dark keeps chrome close to the conversation
    /// surface so selection and content, rather than panel fills, lead the eye.
    pub fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette {
                base: 0x101113,
                surface: 0x17191c,
                chat: 0x0d0e10,
                selected: 0x23262b,
                border: 0x26292e,
                text: 0xe6e8ec,
                prose: 0xc9ccd2,
                muted: 0x92979f,
                disabled: 0x595e66,
                accent: 0x60a5fa,
                primary: 0x3b82f6,
                primary_hover: 0x2563eb,
                primary_pressed: 0x1d4ed8,
                on_primary: 0xffffff,
                warning: 0xe8b86d,
                error: 0xf87171,
                success: 0x43c59e,
                busy: 0xc084fc,
                user: 0x1b1e23,
                code_inline: 0x1e2024,
                code_block: 0x08090b,
                code_header: 0x16181b,
                selection: 0x3b82f659,
                shadow: 0x000000,
                shadow_opacity: 0.32,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
            Self::Light => Palette {
                base: 0xf4f4f5,
                surface: 0xffffff,
                chat: 0xfafafa,
                selected: 0xdbeafe,
                border: 0xe4e4e7,
                text: 0x18181b,
                prose: 0x27272a,
                muted: 0x65656e,
                disabled: 0xa1a1aa,
                accent: 0x1d4ed8,
                primary: 0x2563eb,
                primary_hover: 0x1d4ed8,
                primary_pressed: 0x1e40af,
                on_primary: 0xffffff,
                warning: 0x946000,
                error: 0xdc2626,
                success: 0x16a34a,
                busy: 0x9333ea,
                user: 0xe9eef9,
                code_inline: 0xeceef2,
                code_block: 0xf4f4f5,
                code_header: 0xeaeaed,
                selection: 0x2563eb38,
                shadow: 0x18181b,
                shadow_opacity: 0.10,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
            Self::Nord => Palette {
                base: 0x2e3440,
                surface: 0x3b4252,
                chat: 0x252a33,
                selected: 0x4c566a,
                // nord2: one step above the card surface (nord1), so dividers
                // inside cards stay visible; nord1 itself vanished on cards.
                border: 0x434c5e,
                text: 0xeceff4,
                prose: 0xd8dee9,
                muted: 0x8eabc7,
                // Nord's brightened comment tone: nord3 (0x4c566a) fell to
                // ~1.9:1 on the chat surface, faintest of the three themes.
                disabled: 0x616e88,
                accent: 0x88c0d0,
                primary: 0x81a1c1,
                primary_hover: 0x7395b8,
                primary_pressed: 0x6589ad,
                on_primary: 0x2e3440,
                warning: 0xebcb8b,
                error: 0xbf616a,
                success: 0xa3be8c,
                busy: 0xb48ead,
                user: 0x303843,
                code_inline: 0x323947,
                code_block: 0x1f232b,
                code_header: 0x2b313c,
                selection: 0x88c0d03d,
                shadow: 0x171b22,
                shadow_opacity: 0.28,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
            // Omarchy tokyo-night: background 1a1b26 is the conversation,
            // dark_background the chrome, lighter_background raised cards.
            Self::TokyoNight => Palette {
                base: 0x13141c,
                surface: 0x24283b,
                chat: 0x1a1b26,
                selected: 0x292e42,
                border: 0x2f344a,
                text: 0xc0caf5,
                prose: 0xa9b1d6,
                muted: 0x787fa6,
                disabled: 0x565f89,
                accent: 0x7aa2f7,
                primary: 0x7aa2f7,
                primary_hover: 0x6d95ea,
                primary_pressed: 0x5f87dc,
                on_primary: 0x1a1b26,
                warning: 0xe0af68,
                error: 0xf7768e,
                success: 0x9ece6a,
                busy: 0xbb9af7,
                user: 0x1f2335,
                code_inline: 0x24283b,
                code_block: 0x13141c,
                code_header: 0x1d1f2c,
                selection: 0x7aa2f738,
                shadow: 0x0e0e14,
                shadow_opacity: 0.34,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
            // Omarchy catppuccin (Mocha). Selection is Overlay 2 at ~30%, as
            // Catppuccin's style guide specifies; busy is Mocha's mauve.
            Self::Catppuccin => Palette {
                base: 0x161622,
                surface: 0x313244,
                chat: 0x1e1e2e,
                selected: 0x45475a,
                border: 0x3e4053,
                text: 0xcdd6f4,
                prose: 0xbac2de,
                muted: 0x9399b2,
                disabled: 0x6c7086,
                accent: 0x89b4fa,
                primary: 0x89b4fa,
                primary_hover: 0x7aa7f0,
                primary_pressed: 0x6b99e4,
                on_primary: 0x1e1e2e,
                warning: 0xf9e2af,
                error: 0xf38ba8,
                success: 0xa6e3a1,
                busy: 0xcba6f7,
                user: 0x28283b,
                code_inline: 0x313244,
                code_block: 0x161622,
                code_header: 0x232334,
                selection: 0x9399b245,
                shadow: 0x101019,
                shadow_opacity: 0.34,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
            // Omarchy gruvbox (the Material variant: d4be98 on 282828).
            Self::Gruvbox => Palette {
                base: 0x1e1e1e,
                surface: 0x3c3836,
                chat: 0x282828,
                selected: 0x504945,
                border: 0x48423f,
                text: 0xddc7a1,
                prose: 0xd4be98,
                muted: 0xa89984,
                disabled: 0x7c6f64,
                accent: 0x7daea3,
                primary: 0x7daea3,
                primary_hover: 0x6fa095,
                primary_pressed: 0x619287,
                on_primary: 0x1e1e1e,
                warning: 0xd8a657,
                error: 0xea6962,
                success: 0xa9b665,
                busy: 0xd3869b,
                user: 0x32302f,
                code_inline: 0x3c3836,
                code_block: 0x1e1e1e,
                code_header: 0x2a2827,
                selection: 0x7daea334,
                shadow: 0x161616,
                shadow_opacity: 0.36,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
            // Omarchy everforest (dark medium). Headings brighten fg d3c6aa,
            // which is prose here; dark_foreground is too faint for metadata,
            // so quiet text uses Everforest's grey1.
            Self::Everforest => Palette {
                base: 0x21272c,
                surface: 0x343f44,
                chat: 0x2d353b,
                selected: 0x3d484d,
                border: 0x475258,
                text: 0xe0d5bc,
                prose: 0xd3c6aa,
                muted: 0x9da9a0,
                disabled: 0x859289,
                accent: 0x7fbbb3,
                primary: 0xa7c080,
                primary_hover: 0x99b273,
                primary_pressed: 0x8ba466,
                on_primary: 0x2d353b,
                warning: 0xdbbc7f,
                error: 0xe67e80,
                success: 0xa7c080,
                busy: 0xd699b6,
                user: 0x343f44,
                code_inline: 0x3a464c,
                code_block: 0x232a2e,
                code_header: 0x283035,
                selection: 0x7fbbb32e,
                shadow: 0x181d20,
                shadow_opacity: 0.32,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
            // Omarchy catppuccin-latte on its eff1f5 background. Text, green
            // and yellow are darkened from Latte's own values, which fall
            // short of 7:1 / 3:1 on these light surfaces.
            Self::CatppuccinLatte => Palette {
                base: 0xf3f4f7,
                surface: 0xfbfbfd,
                chat: 0xeff1f5,
                selected: 0xccd0da,
                border: 0xd3d6df,
                text: 0x41445e,
                prose: 0x4c4f69,
                muted: 0x6c6f85,
                disabled: 0x8c8fa1,
                accent: 0x1e66f5,
                primary: 0x1e66f5,
                primary_hover: 0x1a5ad9,
                primary_pressed: 0x164ebd,
                on_primary: 0xffffff,
                warning: 0xa65f00,
                error: 0xd20f39,
                success: 0x2f7d1f,
                busy: 0x8839ef,
                user: 0xe2e7f5,
                code_inline: 0xe3e6ec,
                code_block: 0xe6e9ef,
                code_header: 0xdce0e8,
                selection: 0x1e66f530,
                shadow: 0x4c4f69,
                shadow_opacity: 0.12,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
            },
        }
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(&self)?)?;
        Ok(())
    }
}

pub fn preference_path() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs.home_dir().join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs.home_dir().join(".config"))
    };
    Some(base.join("workspacer/native-theme.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let c = ((color >> shift) & 0xff) as f64 / 255.;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn quiet_text_stays_legible_on_the_chat_surface() {
        // Timestamps and durations use `disabled`; keep every theme readable.
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            let ratio = contrast(p.disabled, p.chat);
            assert!(ratio >= 2.4, "{appearance:?} disabled contrast {ratio:.2}");
            assert!(
                contrast(p.muted, p.chat) > ratio,
                "{appearance:?} muted ≤ disabled"
            );
        }
    }

    /// `rgba` (0xRRGGBBAA) composited over an opaque `under`.
    fn over(rgba: u32, under: u32) -> u32 {
        let alpha = (rgba & 0xff) as f64 / 255.;
        let mix = |shift: u32| {
            let top = ((rgba >> (shift + 8)) & 0xff) as f64;
            let bottom = ((under >> shift) & 0xff) as f64;
            ((top * alpha + bottom * (1. - alpha)).round() as u32) << shift
        };
        mix(16) | mix(8) | mix(0)
    }

    #[test]
    fn selected_text_stays_readable_on_every_surface() {
        // Selection is painted under the glyphs, translucent, so text keeps
        // its own color; it must still read against the tinted background
        // and the tint must be visible against the surface it sits on.
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            let alpha = p.selection & 0xff;
            assert!(
                (0x20..0xa0).contains(&alpha),
                "{appearance:?} selection alpha {alpha:#x}"
            );
            for (name, fill) in [
                ("chat", p.chat),
                ("code block", p.code_block),
                ("user", p.user),
                ("surface", p.surface),
            ] {
                let tinted = over(p.selection, fill);
                for (text_name, text) in [("text", p.text), ("prose", p.prose)] {
                    let ratio = contrast(text, tinted);
                    assert!(
                        ratio >= 4.5,
                        "{appearance:?} {text_name} on selected {name}: {ratio:.2}"
                    );
                }
                assert!(
                    contrast(tinted, fill) >= 1.15,
                    "{appearance:?} selection invisible on {name}: {:.2}",
                    contrast(tinted, fill)
                );
            }
        }
    }

    /// Shortfalls the original Dark/Light/Nord palettes shipped with, kept
    /// so this change does not restyle them. Each still has its own floor;
    /// the Omarchy ports have none of these exceptions.
    const BASELINE_EXCEPTIONS: [(Appearance, &str, f64); 3] = [
        // White on the brand blue button.
        (Appearance::Dark, "primary label", 3.6),
        (Appearance::Light, "success on base", 2.95),
        // Nord's aurora red on nord1 cards.
        (Appearance::Nord, "error on surface", 2.4),
    ];

    #[test]
    fn every_theme_meets_core_contrast() {
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            assert_eq!(
                appearance.is_dark(),
                luminance(p.chat) < 0.4,
                "{appearance:?} mode disagrees with its conversation surface"
            );
            let check = |name: String, ratio: f64, required: f64| match BASELINE_EXCEPTIONS
                .iter()
                .find(|(a, n, _)| *a == appearance && *n == name)
            {
                Some((_, _, floor)) => {
                    assert!(
                        ratio >= *floor,
                        "{appearance:?} {name}: {ratio:.2} < {floor}"
                    )
                }
                None => assert!(
                    ratio >= required,
                    "{appearance:?} {name}: {ratio:.2} < {required}"
                ),
            };
            for (name, fill) in [("chat", p.chat), ("surface", p.surface), ("base", p.base)] {
                for (tone, color, required) in [
                    ("text", p.text, 7.),
                    ("prose", p.prose, 4.5),
                    ("muted", p.muted, 3.),
                    ("accent", p.accent, 3.),
                    ("warning", p.warning, 3.),
                    ("error", p.error, 3.),
                    ("success", p.success, 3.),
                ] {
                    check(format!("{tone} on {name}"), contrast(color, fill), required);
                }
            }
            check(
                "primary label".into(),
                contrast(p.on_primary, p.primary),
                4.5,
            );
        }
    }

    #[test]
    fn omarchy_ports_keep_their_signature_colors() {
        // Spot-check against /usr/share/omarchy/themes/<slug>/colors.toml.
        for (appearance, background, accent, red, green) in [
            (
                Appearance::TokyoNight,
                0x1a1b26,
                0x7aa2f7,
                0xf7768e,
                0x9ece6a,
            ),
            (
                Appearance::Catppuccin,
                0x1e1e2e,
                0x89b4fa,
                0xf38ba8,
                0xa6e3a1,
            ),
            (Appearance::Gruvbox, 0x282828, 0x7daea3, 0xea6962, 0xa9b665),
            (
                Appearance::Everforest,
                0x2d353b,
                0x7fbbb3,
                0xe67e80,
                0xa7c080,
            ),
            // Latte's green (40a02b) is 2.9:1 on its base; darkened to pass.
            (
                Appearance::CatppuccinLatte,
                0xeff1f5,
                0x1e66f5,
                0xd20f39,
                0x2f7d1f,
            ),
        ] {
            let p = appearance.palette();
            assert_eq!(
                (p.chat, p.accent, p.error, p.success),
                (background, accent, red, green),
                "{appearance:?}"
            );
        }
        let slugs: Vec<_> = Appearance::ALL[3..]
            .iter()
            .map(|a| serde_json::to_string(a).unwrap())
            .collect();
        assert_eq!(
            slugs,
            [
                "\"tokyo-night\"",
                "\"catppuccin\"",
                "\"gruvbox\"",
                "\"everforest\"",
                "\"catppuccin-latte\""
            ]
        );
    }

    #[test]
    fn card_dividers_show_on_every_surface() {
        // Rows inside cards and panels are separated by `border` drawn on
        // `surface`; equal colors (Nord once) make every divider vanish.
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            for (name, fill) in [("surface", p.surface), ("chat", p.chat), ("base", p.base)] {
                assert!(
                    contrast(p.border, fill) >= 1.1,
                    "{appearance:?} border on {name}: {:.2}",
                    contrast(p.border, fill)
                );
            }
        }
    }

    #[test]
    fn preference_round_trip_and_invalid_input() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("wks-theme-test-{}-{nonce}", std::process::id()));
        let path = dir.join("native-theme.json");
        assert_eq!(Appearance::load(&path).unwrap(), Appearance::Dark);
        for appearance in Appearance::ALL {
            appearance.save(&path).unwrap();
            assert_eq!(Appearance::load(&path).unwrap(), appearance);
        }
        std::fs::write(&path, b"\"unknown-theme\"").unwrap();
        assert!(Appearance::load(&path).is_err());
        std::fs::write(&path, b"{").unwrap();
        assert!(Appearance::load(&path).is_err());
        assert!(Appearance::Nord.save(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
