//! Per-agent markdown scratchpads, persisted to
//! `~/.config/workspacer/tui-notes.json`.
//!
//! Keyed by **cwd** (like [`crate::names`]) so a note belongs to the project,
//! surviving the session ending / a respawn / a daemon restart. A missing or
//! malformed file degrades to an empty map.

use std::collections::HashMap;
use std::path::PathBuf;

pub fn load() -> HashMap<String, String> {
    read().unwrap_or_default()
}

fn read() -> Option<HashMap<String, String>> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist the map, best-effort.
pub fn save(notes: &HashMap<String, String>) {
    let Some(path) = path() else { return };
    if let Ok(text) = serde_json::to_string_pretty(notes) {
        let _ = std::fs::write(path, text);
    }
}

fn path() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    Some(dirs.config_dir().join("workspacer").join("tui-notes.json"))
}
