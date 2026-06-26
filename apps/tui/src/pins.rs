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

/// Persist the pinned cwds, best-effort (a write failure is silent — the
/// in-memory pins still hold for this session).
pub fn save(cwds: &[String]) {
    let Some(path) = path() else { return };
    if let Ok(text) = serde_json::to_string_pretty(cwds) {
        let _ = std::fs::write(path, text);
    }
}

fn path() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    Some(dirs.config_dir().join("workspacer").join("tui-pins.json"))
}
