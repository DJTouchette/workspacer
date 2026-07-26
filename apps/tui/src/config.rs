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
//!   "colors": { "accent": "#88c0d0", "warn": "yellow" },
//!   "transport": "stream"
//! }
//! ```

use serde::Deserialize;
use std::collections::HashMap;

use crate::keys::{Chord, Context, Keymap};
use crate::theme::{self, Theme};

/// Which transport a Claude session the TUI *spawns* runs on. Only ever applies
/// to new sessions: an existing one reports its own transport on the wire
/// (`Agent::transport`) and every surface follows that, because a running
/// process's transport can't be reinterpreted after the fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// Headless stream-json through claudemon's adapter — structured events, no
    /// PTY, so the chat view is transcript-only. The default: it is what the
    /// desktop defaults to, and it makes the TUI's own spawns match what the
    /// rest of the product does rather than depending on which way the TUI
    /// happens to be talking to the daemon.
    #[default]
    Stream,
    /// The classic PTY TUI. Choose this if you want the terminal view for
    /// sessions you start from here — a stream session has no PTY to show.
    Pty,
}

impl Transport {
    /// The wire spelling claudemon and the hub both use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Pty => "pty",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "stream" => Some(Self::Stream),
            "pty" => Some(Self::Pty),
            _ => None,
        }
    }
}

/// Resolved, ready-to-use config.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub theme: Theme,
    pub keymap: Keymap,
    pub transport: Transport,
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
    /// The leader chord for multi-key bindings / the which-key menu.
    /// Defaults to `space`. e.g. `"leader": ","`.
    #[serde(default)]
    leader: Option<String>,
    /// Keybinding overrides, keyed by context name then chord(-sequence) →
    /// action. A sequence is whitespace-separated, and `<leader>` expands to the
    /// leader chord — e.g. `{"list": {"x": "quit"}, "global": {"<leader> x": "quit"}}`.
    #[serde(default)]
    keys: HashMap<String, HashMap<String, String>>,
    /// Transport for Claude sessions this TUI spawns: `"stream"` (default) or
    /// `"pty"`. Existing sessions always keep their own.
    #[serde(default)]
    transport: Option<String>,
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

        // A custom leader rebuilds the defaults around it (so the which-key
        // menu and `<leader>` overrides all hang off the same key); a bad value
        // falls back to the default leader rather than failing.
        let mut keymap = match self.leader.as_deref() {
            Some(s) => match Chord::parse(s) {
                Some(leader) => Keymap::with_leader(leader),
                None => {
                    eprintln!("wks-tui: bad leader {s:?} in tui.json — using default");
                    Keymap::default()
                }
            },
            None => Keymap::default(),
        };
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

        // An unknown value falls back to the default rather than refusing to
        // start, like every other field here.
        let transport = match self.transport.as_deref() {
            Some(s) => Transport::parse(s).unwrap_or_else(|| {
                eprintln!("wks-tui: unknown transport {s:?} in tui.json — using stream");
                Transport::default()
            }),
            None => Transport::default(),
        };

        Config {
            theme,
            keymap,
            transport,
        }
    }
}

/// Load and resolve the config, falling back to defaults on any problem.
pub fn load() -> Config {
    read_file().unwrap_or_default().resolve()
}

/// The hub bus token the desktop app persists at `~/.config/workspacer/remote-token`
/// (see `hubDaemon.ts`). When the desktop is running it owns the hub and guards
/// `/bus` with this token, so a TUI joining that bus must present it or the
/// WebSocket handshake is rejected with 401. Returns None when the file is
/// absent (e.g. the desktop has never run / remote sharing off) — in which case
/// the TUI either spawns its own token-less hub or talks to claudemon directly.
/// Presenting this token to a token-less hub is harmless: the hub ignores it.
pub fn hub_token() -> Option<String> {
    let dirs = directories::BaseDirs::new()?;
    let path = dirs.config_dir().join("workspacer").join("remote-token");
    let token = std::fs::read_to_string(path).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
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

    /// Spawning on stream is the default so the TUI's own sessions match the
    /// rest of the product instead of depending on which way it reached the
    /// daemon. `pty` is there for anyone who wants the terminal view for
    /// sessions they start here — a stream session has no PTY to show.
    #[test]
    fn transport_defaults_to_stream_and_accepts_pty() {
        assert_eq!(resolve("{}").transport, Transport::Stream);
        assert_eq!(
            resolve(r#"{"transport":"stream"}"#).transport,
            Transport::Stream
        );
        assert_eq!(resolve(r#"{"transport":"pty"}"#).transport, Transport::Pty);
        assert_eq!(
            resolve(r#"{"transport":" pty "}"#).transport,
            Transport::Pty
        );
    }

    #[test]
    fn a_bad_transport_falls_back_rather_than_failing_startup() {
        assert_eq!(
            resolve(r#"{"transport":"headless"}"#).transport,
            Transport::Stream
        );
        assert_eq!(resolve(r#"{"transport":""}"#).transport, Transport::Stream);
    }

    #[test]
    fn transport_wire_spelling_matches_the_daemon() {
        assert_eq!(Transport::Stream.as_str(), "stream");
        assert_eq!(Transport::Pty.as_str(), "pty");
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
        assert_eq!(
            cfg.keymap.action(Context::List, Chord::parse("q").unwrap()),
            None
        );
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
        assert_eq!(
            cfg.keymap.action(Context::List, Chord::parse("z").unwrap()),
            None
        );
    }
}
