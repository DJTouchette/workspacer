//! User config for the TUI, read from `~/.config/workspacer/tui.json` (the same
//! `workspacer` config dir the Electron app and `profiles.rs` use). Everything
//! is optional with sane defaults, and a malformed or missing file degrades to
//! defaults rather than failing startup — a TUI you can't launch because of a
//! typo'd config is worse than one that ignores it.
//!
//! Example `tui.json`:
//! ```json
//! {
//!   "theme": "nord",
//!   "colors": { "accent": "#88c0d0", "warn": "yellow" }
//! }
//! ```

use serde::Deserialize;
use std::collections::HashMap;

use crate::keys::{Context, Keymap};
use crate::theme::{self, Theme};

/// Resolved, ready-to-use config.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub theme: Theme,
    pub keymap: Keymap,
}

/// The on-disk shape. All fields optional and defaulted so partial files (and
/// future keys this version doesn't know) parse cleanly.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    /// Named preset (`default` / `nord` / `gruvbox` / `ansi`). Unknown → default.
    #[serde(default)]
    theme: Option<String>,
    /// Per-role color overrides applied on top of the chosen preset.
    #[serde(default)]
    colors: HashMap<String, String>,
    /// Keybinding overrides, keyed by context name then chord → action.
    /// e.g. `{"list": {"x": "quit"}, "global": {"f1": "help"}}`.
    #[serde(default)]
    keys: HashMap<String, HashMap<String, String>>,
}

impl RawConfig {
    /// Resolve the raw file into a usable [`Config`].
    fn resolve(self) -> Config {
        let mut theme = self
            .theme
            .as_deref()
            .and_then(Theme::preset)
            .unwrap_or_default();
        for (role, value) in &self.colors {
            if let Some(color) = theme::parse_color(value) {
                theme.set_role(role, color);
            }
        }

        let mut keymap = Keymap::default();
        for (ctx_name, binds) in &self.keys {
            let Some(ctx) = Context::from_name(ctx_name) else {
                eprintln!("wks-tui: unknown key context {ctx_name:?} in tui.json — skipped");
                continue;
            };
            for (chord, action) in binds {
                if !keymap.set(ctx, chord, action) {
                    eprintln!(
                        "wks-tui: bad binding {ctx_name}.{chord:?} = {action:?} in tui.json — skipped"
                    );
                }
            }
        }

        Config { theme, keymap }
    }
}

/// Load and resolve the config, falling back to defaults on any problem.
pub fn load() -> Config {
    read_file().unwrap_or_default().resolve()
}

fn read_file() -> Option<RawConfig> {
    let dirs = directories::BaseDirs::new()?;
    let path = dirs.config_dir().join("workspacer").join("tui.json");
    let text = std::fs::read_to_string(path).ok()?;
    // A broken config shouldn't brick the TUI — warn-and-default instead.
    match serde_json::from_str(&text) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            eprintln!("wks-tui: ignoring malformed tui.json: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn resolve(json: &str) -> Config {
        let raw: RawConfig = serde_json::from_str(json).unwrap();
        raw.resolve()
    }

    #[test]
    fn empty_config_is_default_theme() {
        let cfg = resolve("{}");
        assert_eq!(cfg.theme, Theme::default());
    }

    #[test]
    fn named_preset_selected() {
        let cfg = resolve(r#"{"theme":"nord"}"#);
        assert_eq!(cfg.theme, Theme::preset("nord").unwrap());
    }

    #[test]
    fn unknown_preset_falls_back_to_default() {
        let cfg = resolve(r#"{"theme":"chartreuse"}"#);
        assert_eq!(cfg.theme, Theme::default());
    }

    #[test]
    fn color_overrides_apply_on_top_of_preset() {
        let cfg = resolve(r##"{"theme":"nord","colors":{"accent":"#010203","warn":"yellow"}}"##);
        assert_eq!(cfg.theme.accent, Color::Rgb(1, 2, 3));
        assert_eq!(cfg.theme.warn, Color::Yellow);
        // Untouched roles keep the preset value.
        assert_eq!(cfg.theme.ok, Theme::preset("nord").unwrap().ok);
    }

    #[test]
    fn bad_color_value_is_ignored() {
        let cfg = resolve(r#"{"colors":{"accent":"not-a-color"}}"#);
        assert_eq!(cfg.theme.accent, Theme::default().accent);
    }

    #[test]
    fn unknown_top_level_keys_are_ignored() {
        // Forward-compat: a future unknown section won't break this version.
        let cfg = resolve(r#"{"theme":"gruvbox","future_section":{"a":1}}"#);
        assert_eq!(cfg.theme, Theme::preset("gruvbox").unwrap());
    }

    #[test]
    fn key_overrides_apply() {
        use crate::keys::{Action, Chord, Context};
        let cfg = resolve(r#"{"keys":{"list":{"x":"quit","q":"none"}}}"#);
        assert_eq!(
            cfg.keymap.action(Context::List, Chord::parse("x").unwrap()),
            Some(Action::Quit)
        );
        // "none" unbinds the default.
        assert_eq!(cfg.keymap.action(Context::List, Chord::parse("q").unwrap()), None);
        // Untouched defaults survive.
        assert_eq!(
            cfg.keymap.action(Context::List, Chord::parse("j").unwrap()),
            Some(Action::SelectNext)
        );
    }

    #[test]
    fn bad_bindings_skipped_not_fatal() {
        use crate::keys::{Chord, Context};
        // Unknown context, unparseable chord, unknown action — all ignored.
        let cfg = resolve(
            r#"{"keys":{"nope":{"a":"quit"},"list":{"boguskey":"quit","z":"frobnicate"}}}"#,
        );
        // The valid default for "z" context-free: z isn't a default in list, so None.
        assert_eq!(cfg.keymap.action(Context::List, Chord::parse("z").unwrap()), None);
    }
}
