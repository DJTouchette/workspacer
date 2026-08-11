//! Shared helper for the TUI's small JSON stores (names, pins, notes).
//!
//! They all live under `$XDG_CONFIG_HOME/workspacer/`, a directory the Electron
//! app creates and the standalone `wks-tui` never did. On a TUI-only machine
//! every save failed with ENOENT — permanently, not transiently — while the UI
//! toasted "Renamed". Creating the parent is one line; not creating it cost
//! every rename, pin and hand-typed note the user ever made.

use std::path::Path;

/// The toast text for a store write.
///
/// Exists so the decision "does this UI claim success for a write that failed?"
/// is one function with its own test, rather than four call sites each free to
/// re-introduce a fixed success string. `crate::names::save(&..); set_toast("Renamed")`
/// is precisely what made a permanent ENOENT invisible.
pub fn save_toast(ok_msg: &str, what: &str, res: Result<(), String>) -> String {
    match res {
        Ok(()) => ok_msg.to_string(),
        Err(e) => format!("{what} NOT saved: {e}"),
    }
}

/// Create `path`'s parent directory if it does not exist yet.
pub fn ensure_parent(path: &Path) -> Result<(), String> {
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// A directory under the crate's own `target/` (never /tmp — see
    /// testenv.rs), wiped on entry. No `XDG_CONFIG_HOME` mutation, so these
    /// cannot race the App suites that pin it once per process.
    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("store-tests")
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The `workspacer` component is deliberately NOT created: that is the
        // state of a machine which only ever ran the standalone `wks-tui`.
        dir.join("workspacer")
    }

    #[test]
    fn names_save_creates_the_directory_nothing_else_creates() {
        let cfg = scratch("names");
        assert!(
            !cfg.exists(),
            "the test must start with no workspacer config dir"
        );
        let file = cfg.join("tui-names.json");

        let mut names = HashMap::new();
        names.insert("/work/repo".to_string(), "backend".to_string());
        crate::names::save_at(&file, &names)
            .expect("a machine that has never run the Electron app must still be able to rename");

        assert!(file.is_file(), "the rename never reached disk");
        assert_eq!(
            crate::names::load_at(&file),
            names,
            "the rename did not survive a reload"
        );
    }

    #[test]
    fn pins_and_notes_create_it_too() {
        let cfg = scratch("pinsnotes");
        crate::pins::save_at(&cfg.join("tui-pins.json"), &["/work/a".to_string()]).expect("pins");
        assert!(cfg.join("tui-pins.json").is_file());

        let mut notes = HashMap::new();
        notes.insert("/work/a".to_string(), "# scratch".to_string());
        crate::notes::save_at(&cfg.join("tui-notes.json"), &notes).expect("notes");
        assert!(cfg.join("tui-notes.json").is_file());
    }

    /// The other half: a write that genuinely cannot land must REPORT, so the
    /// caller's toast says so instead of saying "Renamed".
    #[cfg(unix)]
    #[test]
    fn a_write_that_cannot_land_returns_a_reason() {
        use std::os::unix::fs::PermissionsExt;
        let cfg = scratch("readonly");
        let parent = cfg.parent().unwrap().to_path_buf();
        let mut perms = std::fs::metadata(&parent).unwrap().permissions();
        perms.set_mode(0o500);
        std::fs::set_permissions(&parent, perms).unwrap();

        let err = crate::names::save_at(&cfg.join("tui-names.json"), &HashMap::new())
            .expect_err("a save into an unwritable directory must not report success");
        assert!(!err.is_empty(), "a failed save must carry a reason");

        let mut perms = std::fs::metadata(&parent).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&parent, perms).unwrap();
    }

    #[test]
    fn a_failed_write_never_produces_the_success_toast() {
        assert_eq!(save_toast("Renamed", "rename", Ok(())), "Renamed");
        let failed = save_toast("Renamed", "rename", Err("ENOENT".into()));
        assert_ne!(failed, "Renamed", "a failed save must not toast success");
        assert!(failed.contains("NOT saved"), "got {failed:?}");
        assert!(
            failed.contains("ENOENT"),
            "the reason must survive: {failed:?}"
        );
    }

    #[test]
    fn ensure_parent_is_idempotent() {
        let cfg = scratch("idem");
        let file = cfg.join("nested").join("x.json");
        ensure_parent(&file).unwrap();
        ensure_parent(&file).unwrap();
        assert!(file.parent().unwrap().is_dir());
    }
}
