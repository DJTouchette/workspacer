//! Persistent harpoon pins, saved to `~/.config/workspacer/tui-pins.json`.
//!
//! Pins are stored as an **ordered list of cwds**, not session ids — session
//! ids are ephemeral (a fresh run mints a new one; a restart replays old ones as
//! stopped), whereas the cwd is the stable identity of "this project's agent"
//! (the same key [`crate::names`] and [`crate::notes`] use). On load the cwds
//! are resolved back to whatever live session is in each directory. A missing or
//! malformed file degrades to no pins (they're a convenience, never load-bearing).

use std::path::PathBuf;

/// Load the saved ordered list of pinned cwds (empty on any problem).
pub fn load() -> Vec<String> {
    read().unwrap_or_default()
}

fn read() -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist the pinned cwds. Returns `Err(message)` on failure — see
/// [`crate::names::save`] for why a silent write here lost every pin on a
/// TUI-only machine.
pub fn save(cwds: &[String]) -> Result<(), String> {
    let path = path().ok_or_else(|| "no config directory".to_string())?;
    save_at(&path, cwds)
}

/// Test seam — see [`crate::names::save_at`].
pub(crate) fn save_at(path: &std::path::Path, cwds: &[String]) -> Result<(), String> {
    crate::store::ensure_parent(path)?;
    let text = serde_json::to_string_pretty(cwds).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn path() -> Option<PathBuf> {
    Some(crate::config::config_dir()?.join("tui-pins.json"))
}
