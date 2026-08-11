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

/// Persist the map. Returns `Err(message)` rather than swallowing the failure:
/// `$XDG_CONFIG_HOME/workspacer/` is created by the Electron app, and a machine
/// that only ever ran the standalone `wks-tui` has no such directory — so the
/// write failed with ENOENT on EVERY save, forever, while the caller showed a
/// success toast and the rename was gone on the next launch. Creating the
/// directory here is the other half of the fix.
pub fn save(names: &HashMap<String, String>) -> Result<(), String> {
    let path = path().ok_or_else(|| "no config directory".to_string())?;
    save_at(&path, names)
}

/// `save` with the destination given, so a test can exercise the real
/// create-then-write without mutating the process's `XDG_CONFIG_HOME`.
pub(crate) fn save_at(
    path: &std::path::Path,
    names: &HashMap<String, String>,
) -> Result<(), String> {
    crate::store::ensure_parent(path)?;
    let text = serde_json::to_string_pretty(names).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// `load` with the source given (test seam, mirroring [`save_at`]).
///
/// Test-only, and marked so: `save_at` has a production caller (`save`), this
/// does not, and `clippy --all-targets -D warnings` fails the bin build on the
/// dead code rather than the test build that uses it.
#[cfg(test)]
pub(crate) fn load_at(path: &std::path::Path) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn path() -> Option<PathBuf> {
    Some(crate::config::config_dir()?.join("tui-names.json"))
}
