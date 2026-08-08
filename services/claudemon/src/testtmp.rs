//! Where throwaway test databases live, and why they live somewhere.
//!
//! Three test helpers — `store::tests::tempfile_path`, `daemon::api::tests::
//! test_state` and `daemon::mcp_ask::tests::test_state` — each created a
//! uniquely-named sqlite file directly in `std::env::temp_dir()` and never
//! removed it. One suite run left several hundred `claudemon-*-test-*.db`
//! (plus `-wal` and `-shm`) files loose in /tmp, and they accumulated across
//! every run: 1034 of them were sitting there when this module was written.
//!
//! On this project's machines /tmp is a per-user-quota tmpfs, and a full quota
//! does not merely fail a test — it stops the shell being able to exec, which
//! has already ended one hardening round early. Debris that grows without
//! bound, in the one directory that cannot afford it, is worth fifteen lines.
//!
//! Two things change here, and deliberately not a third:
//!
//!   - every test db goes in ONE directory, so the debris is a single `rm -rf`
//!     rather than a thousand loose entries interleaved with everyone else's;
//!   - that directory is pruned of anything older than an hour, once per
//!     process, so consecutive runs replace rather than accumulate.
//!
//! What is NOT done: deleting each file when its test ends. That would need a
//! guard threaded through ~40 call sites, and an hour-old prune already makes
//! the footprint bounded by one run instead of by uptime.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// How long a leftover test database is kept before the next run prunes it.
/// Long enough that a concurrently running test binary (cargo runs several in
/// parallel) never has its file removed underneath it; short enough that the
/// directory holds one run's worth rather than one machine-lifetime's.
const PRUNE_AFTER: Duration = Duration::from_secs(60 * 60);

/// The single directory every throwaway test database is created in.
pub fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join("claudemon-test-dbs");
    let _ = std::fs::create_dir_all(&dir);
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| prune_older_than(&dir, PRUNE_AFTER));
    dir
}

/// A fresh, uniquely named database path inside that directory.
pub fn db_path(prefix: &str) -> PathBuf {
    dir().join(format!("{prefix}-{}.db", uuid::Uuid::new_v4()))
}

/// The sweep. Best-effort throughout: a file another process still holds may
/// fail to unlink, and that is not worth failing a test run over.
pub fn prune_older_than(dir: &Path, age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if matches!(now.duration_since(modified), Ok(since) if since >= age) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_unique_and_inside_the_one_pruned_directory() {
        let a = db_path("unit");
        let b = db_path("unit");
        assert_ne!(a, b, "two test dbs collided");
        assert_eq!(a.parent(), b.parent());
        assert_eq!(
            a.parent().unwrap().file_name().unwrap(),
            "claudemon-test-dbs",
            "test databases escaped the directory that gets pruned, so nothing will ever remove them"
        );
    }

    /// The prune has to actually delete, or the directory is just a tidier
    /// place to leak into.
    #[test]
    fn the_prune_removes_what_it_is_asked_to() {
        let d =
            std::env::temp_dir().join(format!("claudemon-prune-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let victim = d.join("old.db");
        std::fs::write(&victim, b"x").unwrap();

        // Age zero: everything already written is older than "no time at all".
        prune_older_than(&d, Duration::ZERO);
        assert!(
            !victim.exists(),
            "the prune left a file it was asked to remove"
        );

        // …and a longer horizon spares a file just written.
        let keeper = d.join("new.db");
        std::fs::write(&keeper, b"x").unwrap();
        prune_older_than(&d, Duration::from_secs(3600));
        assert!(keeper.exists(), "the prune removed a live test db");
        let _ = std::fs::remove_dir_all(&d);
    }
}
