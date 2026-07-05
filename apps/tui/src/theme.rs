//! Color theme for the TUI.
//!
//! `ui.rs` used to hardcode five palette constants; they now live here as a
//! [`Theme`] resolved from the user's config (`~/.config/workspacer/tui.json`).
//! A theme is a named built-in preset plus optional per-role overrides, so a
//! user can pick `nord` and still nudge a single color. Unknown names fall back
//! to the default palette rather than erroring.

use ratatui::style::Color;

/// The semantic color roles the renderer draws with. Every widget references a
/// role, never a literal color, so swapping a theme recolors the whole UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Primary brand / focus color (active tab, prompts, cursor).
    pub accent: Color,
    /// Success / healthy / "claude" turns.
    pub ok: Color,
    /// Needs-attention / waiting / toasts.
    pub warn: Color,
    /// Errors / destructive / disconnected.
    pub bad: Color,
    /// Secondary text, labels, borders.
    pub dim: Color,
    /// Default foreground for body text.
    pub fg: Color,
    /// Background of the selected sidebar row.
    pub selection_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::preset("default").unwrap()
    }
}

impl Theme {
    /// A built-in preset by name, or `None` if the name is unknown. Names are
    /// matched case-insensitively. Keep `BUILTINS` in sync for the help overlay.
    pub fn preset(name: &str) -> Option<Theme> {
        let rgb = Color::Rgb;
        Some(match name.to_lowercase().as_str() {
            // The original /remote-matching palette.
            "default" | "dark" => Theme {
                accent: rgb(110, 168, 254),
                ok: rgb(78, 201, 168),
                warn: rgb(224, 179, 65),
                bad: rgb(224, 108, 117),
                dim: rgb(139, 145, 156),
                fg: Color::Reset,
                selection_bg: rgb(29, 32, 38),
            },
            "nord" => Theme {
                accent: rgb(136, 192, 208),
                ok: rgb(163, 190, 140),
                warn: rgb(235, 203, 139),
                bad: rgb(191, 97, 106),
                dim: rgb(118, 128, 144),
                fg: rgb(216, 222, 233),
                selection_bg: rgb(59, 66, 82),
            },
            "gruvbox" => Theme {
                accent: rgb(131, 165, 152),
                ok: rgb(184, 187, 38),
                warn: rgb(250, 189, 47),
                bad: rgb(251, 73, 52),
                dim: rgb(146, 131, 116),
                fg: rgb(235, 219, 178),
                selection_bg: rgb(60, 56, 54),
            },
            // Uses the terminal's own 16-color palette — respects the user's
            // terminal theme instead of imposing RGB.
            "ansi" | "mono" => Theme {
                accent: Color::Cyan,
                ok: Color::Green,
                warn: Color::Yellow,
                bad: Color::Red,
                dim: Color::DarkGray,
                fg: Color::Reset,
                selection_bg: Color::Indexed(236),
            },
            _ => return None,
        })
    }

    /// Apply a single role override from a parsed color. Unknown role names are
    /// ignored (forward-compat with future config keys).
    pub fn set_role(&mut self, role: &str, color: Color) {
        match role {
            "accent" => self.accent = color,
            "ok" => self.ok = color,
            "warn" => self.warn = color,
            "bad" => self.bad = color,
            "dim" => self.dim = color,
            "fg" => self.fg = color,
            "selection_bg" | "selection" => self.selection_bg = color,
            _ => {}
        }
    }
}

/// Names of the built-in presets, for the help overlay / docs.
pub const BUILTINS: &[&str] = &["default", "nord", "gruvbox", "ansi"];

/// Parse a color from a config string: `#rrggbb` / `rrggbb` hex, an ANSI color
/// name (`red`, `cyan`, `darkgray`…), or `N` / `indexed(N)` for a 256-color
/// index. Returns `None` on anything unrecognised so the caller keeps the
/// preset's value.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    // `#rrggbb`, or a bare `rrggbb` only when every char is a hex digit (so a
    // 6-letter name like "yellow" isn't mistaken for hex and swallowed).
    let hex = s
        .strip_prefix('#')
        .or_else(|| (s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s));
    if let Some(hex) = hex {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
    }
    if let Ok(n) = s.parse::<u8>() {
        return Some(Color::Indexed(n));
    }
    Some(match s.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "white" => Color::White,
        "reset" | "default" => Color::Reset,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_preset_matches_legacy_palette() {
        let t = Theme::default();
        assert_eq!(t.accent, Color::Rgb(110, 168, 254));
        assert_eq!(t.ok, Color::Rgb(78, 201, 168));
        assert_eq!(t.warn, Color::Rgb(224, 179, 65));
        assert_eq!(t.bad, Color::Rgb(224, 108, 117));
        assert_eq!(t.dim, Color::Rgb(139, 145, 156));
        assert_eq!(t.selection_bg, Color::Rgb(29, 32, 38));
    }

    #[test]
    fn unknown_preset_is_none() {
        assert!(Theme::preset("chartreuse").is_none());
        assert!(
            Theme::preset("NORD").is_some(),
            "names are case-insensitive"
        );
    }

    #[test]
    fn parse_hex_with_and_without_hash() {
        assert_eq!(parse_color("#6ea8fe"), Some(Color::Rgb(110, 168, 254)));
        assert_eq!(parse_color("6ea8fe"), Some(Color::Rgb(110, 168, 254)));
        assert_eq!(parse_color("  #ffffff "), Some(Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn parse_named_and_indexed() {
        assert_eq!(parse_color("cyan"), Some(Color::Cyan));
        assert_eq!(parse_color("DarkGray"), Some(Color::DarkGray));
        assert_eq!(parse_color("236"), Some(Color::Indexed(236)));
        assert_eq!(parse_color("nonsense"), None);
        assert_eq!(parse_color("#zzz"), None);
    }

    #[test]
    fn set_role_applies_override() {
        let mut t = Theme::default();
        t.set_role("accent", Color::Rgb(1, 2, 3));
        assert_eq!(t.accent, Color::Rgb(1, 2, 3));
        t.set_role("unknown_role", Color::Red); // ignored, no panic
        assert_eq!(t.accent, Color::Rgb(1, 2, 3));
    }
}
