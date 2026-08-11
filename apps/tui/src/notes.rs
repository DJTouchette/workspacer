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

/// Persist the map. Returns `Err(message)` on failure — see
/// [`crate::names::save`] for why a silent write here lost every note on a
/// TUI-only machine.
pub fn save(notes: &HashMap<String, String>) -> Result<(), String> {
    let path = path().ok_or_else(|| "no config directory".to_string())?;
    save_at(&path, notes)
}

/// Test seam — see [`crate::names::save_at`].
pub(crate) fn save_at(
    path: &std::path::Path,
    notes: &HashMap<String, String>,
) -> Result<(), String> {
    crate::store::ensure_parent(path)?;
    let text = serde_json::to_string_pretty(notes).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn path() -> Option<PathBuf> {
    Some(crate::config::config_dir()?.join("tui-notes.json"))
}
