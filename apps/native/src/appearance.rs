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
    pub muted: u32,
    pub disabled: u32,
    pub accent: u32,
    pub primary: u32,
    pub on_primary: u32,
    pub warning: u32,
    pub success: u32,
    pub busy: u32,
    pub user: u32,
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

    /// Semantic colors from desktop themes.ts; translucent bubbles composited on chat.
    pub fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette {
                base: 0x18181b,
                surface: 0x1e1e21,
                chat: 0x0d0d10,
                selected: 0x2d303c,
                border: 0x2d2d32,
                text: 0xdcdceb,
                muted: 0x8c8c9b,
                disabled: 0x5a5a64,
                accent: 0x60a5fa,
                primary: 0x5078c8,
                on_primary: 0xffffff,
                warning: 0xfacc15,
                success: 0x4ade80,
                busy: 0xc084fc,
                user: 0x141925,
            },
            Self::Light => Palette {
                base: 0xf4f4f5,
                surface: 0xffffff,
                chat: 0xfafafa,
                selected: 0xdbeafe,
                border: 0xe4e4e7,
                text: 0x18181b,
                muted: 0x65656e,
                disabled: 0xa1a1aa,
                accent: 0x1d4ed8,
                primary: 0x2563eb,
                on_primary: 0xffffff,
                warning: 0x946000,
                success: 0x16a34a,
                busy: 0x9333ea,
                user: 0xe9eef9,
            },
            Self::Nord => Palette {
                base: 0x2e3440,
                surface: 0x3b4252,
                chat: 0x252a33,
                selected: 0x4c566a,
                border: 0x3b4252,
                text: 0xeceff4,
                muted: 0x8eabc7,
                disabled: 0x4c566a,
                accent: 0x88c0d0,
                primary: 0x81a1c1,
                on_primary: 0x2e3440,
                warning: 0xebcb8b,
                success: 0xa3be8c,
                busy: 0xb48ead,
                user: 0x303843,
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
