//! Harpoon pins — ONE store shared with the desktop's command layer.
//!
//! Pins are an **ordered list of cwds**, not session ids — session ids are
//! ephemeral (a fresh run mints a new one; a restart replays old ones as
//! stopped), whereas the cwd is the stable identity of "this project's agent"
//! (the same key [`crate::names`] and [`crate::notes`] use). On load the cwds
//! are resolved back to whatever live session is in each directory.
//!
//! The shared truth is `ui.pinnedAgentCwds` in `config.yaml` — the SAME slots
//! the desktop's `prefix m` / `prefix 1-9` use, so harpoon 3 here and
//! `prefix 3` there always mean the same agent. The TUI never writes
//! config.yaml itself (it has two disciplined writers already — desktop TS +
//! the Go brain, mtime-gated); it reads pins out of the `config.get` document
//! and writes them back through the brain's `config.save` (an array under
//! deep-merge is replaced wholesale, so unpinning round-trips).
//!
//! `tui-pins.json` remains as the OFF-BUS store (`--direct`, unreachable hub)
//! and as the migration source: the first time the shared key is found absent
//! while legacy pins exist, they are pushed up. A missing or malformed file
//! degrades to no pins (they're a convenience, never load-bearing).

use std::path::PathBuf;

/// `ui.pinnedAgentCwds` from a whole config document (what `config.get`
/// returns). `None` means the KEY IS ABSENT — never written by any client —
/// which is what gates the one-time legacy migration; an explicit empty list
/// is a real answer ("everything unpinned") and comes back as `Some(vec![])`.
pub fn from_config(doc: &serde_json::Value) -> Option<Vec<String>> {
    let arr = doc.get("ui")?.get("pinnedAgentCwds")?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// The `config.save` partial that makes `cwds` the shared truth.
pub fn config_partial(cwds: &[String]) -> serde_json::Value {
    serde_json::json!({ "ui": { "pinnedAgentCwds": cwds } })
}

/// Load the LEGACY (off-bus) ordered list of pinned cwds (empty on any problem).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_distinguishes_absent_from_empty() {
        // Absent key → None (gates the one-time legacy migration).
        assert_eq!(from_config(&serde_json::json!({})), None);
        assert_eq!(from_config(&serde_json::json!({"ui": {}})), None);
        // Explicit empty → Some(vec![]) — a real "everything unpinned".
        assert_eq!(
            from_config(&serde_json::json!({"ui": {"pinnedAgentCwds": []}})),
            Some(Vec::new())
        );
        // Non-strings are dropped, order preserved.
        assert_eq!(
            from_config(&serde_json::json!({"ui": {"pinnedAgentCwds": ["/a", 7, "/b"]}})),
            Some(vec!["/a".to_string(), "/b".to_string()])
        );
    }

    #[test]
    fn config_partial_is_the_wholesale_array_shape() {
        // deepMerge (both writers) replaces arrays wholesale — the partial
        // must carry the ENTIRE list, so unpinning round-trips.
        assert_eq!(
            config_partial(&["/a".to_string()]),
            serde_json::json!({"ui": {"pinnedAgentCwds": ["/a"]}})
        );
    }
}
