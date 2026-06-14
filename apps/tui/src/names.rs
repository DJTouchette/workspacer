//! TUI-local custom agent names, persisted to
//! `~/.config/workspacer/tui-names.json`.
//!
//! Keyed by **cwd** rather than session id, so a rename sticks across the
//! session ending, a respawn, or a daemon restart — the cwd is the stable
//! identity of "this project's agent". A missing or malformed file degrades to
//! an empty map (renames are a convenience, never load-bearing).

use std::collections::HashMap;
use std::path::PathBuf;

/// Load the saved cwd → name map (empty on any problem).
pub fn load() -> HashMap<String, String> {
    read().unwrap_or_default()
}

fn read() -> Option<HashMap<String, String>> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist the map, best-effort (a write failure is silent — the in-memory map
/// still reflects the rename for this session).
pub fn save(names: &HashMap<String, String>) {
    let Some(path) = path() else { return };
    if let Ok(text) = serde_json::to_string_pretty(names) {
        let _ = std::fs::write(path, text);
    }
}

fn path() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    Some(dirs.config_dir().join("workspacer").join("tui-names.json"))
}
