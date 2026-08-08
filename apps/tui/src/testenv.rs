//! Test-only environment isolation.
//!
//! Both suites that drive the real `App` persist to disk — harpoon pins, agent
//! names, notes — so they redirect `XDG_CONFIG_HOME` before touching anything.
//! Both did it the same way, and the way was wrong in the same way:
//!
//! ```ignore
//! let dir = std::env::temp_dir().join(format!("wks-tui-test-{}", std::process::id()));
//! ```
//!
//! A pid-named directory under /tmp, created and never removed. Nothing fails
//! when it leaks — which is exactly why it kept leaking. /tmp on this machine is
//! a per-user-quota tmpfs, and an earlier round found 213,000 abandoned `wks-*`
//! directories from suites with this same shape; the symptom is not a failing
//! test but `fork: disk quota exceeded`, i.e. the shell losing the ability to
//! start a process at all.
//!
//! The fix is not "remember to clean up" — a static `Once` has no destructor and
//! a test binary has no teardown hook, so there is no honest place to hang one.
//! It is to stop using /tmp: the directory goes under the crate's own `target/`
//! (already gitignored, already what `cargo clean` removes) under a FIXED name,
//! wiped on entry so each run starts empty. One directory, forever, instead of
//! one per process.

use std::path::PathBuf;

/// Points `XDG_CONFIG_HOME` at a private, reused directory and empties it.
///
/// `tag` separates suites that run in the same process tree so one cannot see
/// the other's pins.
pub(crate) fn isolate_config_home(tag: &str) {
    let dir: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-config")
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("XDG_CONFIG_HOME", &dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the change: nothing lands in the system temp dir.
    #[test]
    fn the_isolated_config_home_is_not_in_the_system_temp_dir() {
        isolate_config_home("selfcheck");
        let home = std::env::var("XDG_CONFIG_HOME").expect("XDG_CONFIG_HOME must be set");
        let tmp = std::env::temp_dir();
        assert!(
            !std::path::Path::new(&home).starts_with(&tmp),
            "the test config dir {home} is under the system temp dir {} — /tmp here is a per-user-quota tmpfs and these directories are never removed",
            tmp.display()
        );
        assert!(
            std::path::Path::new(&home).is_dir(),
            "{home} was not created, so the suites would fall back to the real ~/.config"
        );
        // Re-entering must WIPE rather than accumulate: the fixed name is only
        // safe because each run starts empty.
        std::fs::write(std::path::Path::new(&home).join("stale"), b"x").unwrap();
        isolate_config_home("selfcheck");
        assert!(
            !std::path::Path::new(&home).join("stale").exists(),
            "a second call left the previous run's files behind"
        );
    }
}
