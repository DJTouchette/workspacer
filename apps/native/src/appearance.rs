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
    pub shadow: u32,
    pub shadow_opacity: f32,
    pub control_radius: f32,
    pub panel_radius: f32,
    pub composer_radius: f32,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::Nord];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Nord => "Nord",
        }
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
                border: 0x3b4252,
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
                shadow: 0x171b22,
                shadow_opacity: 0.28,
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
