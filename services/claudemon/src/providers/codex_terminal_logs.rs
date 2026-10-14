//! Where Codex background-terminal logs live, and how they are written.
//!
//! Codex streams a terminal's output to us (`item/commandExecution/outputDelta`);
//! the daemon keeps it so `sessions.taskOutput` can serve it. These files sit
//! in the user's own local data dir (`~/.local/share/workspacer/codex-terminals`
//! on Linux, `%LOCALAPPDATA%\workspacer\codex-terminals` on Windows), never in
//! a shared temp dir. On Unix every directory is created `0700` and must be
//! the user's own real directory (no symlink, no foreign owner), and files are
//! created with `create_new` and appended with `O_NOFOLLOW`, matching
//! `background_tasks::read_output`'s hardening.
//!
//! Lifetime: a driver removes its session's directory when it ends, evicted
//! task rows lose their file at once, and [`sweep`] (daemon start) removes any
//! session directory untouched for [`STALE_AFTER`].

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A session directory nobody has written to for this long is a leftover.
pub const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The per-user root, or `None` when the platform names no local data dir.
pub fn root() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    Some(
        base.data_local_dir()
            .join("workspacer")
            .join("codex-terminals"),
    )
}

/// Create `dir` (and missing parents) privately, then prove it is the
/// user's own real directory.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let uid = unsafe { nix::libc::geteuid() };
        if !meta.file_type().is_dir() || meta.uid() != uid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "terminal log directory is not this user's own directory",
            ));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        if !meta.file_type().is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "terminal log directory is not a directory",
            ));
        }
        Ok(())
    }
}

/// Create a new log holding `content`. Refuses an existing file or link.
pub fn create(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    }
    options.open(path)?.write_all(content)
}

/// Append to a log this driver created; never follows a link.
pub fn append(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "terminal log is a link",
        ));
    }
    options.open(path)?.write_all(content)
}

/// Remove one session's logs (best effort).
pub fn remove_session(dir: &Path) {
    if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir()) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Newest modification time of a session directory and its files.
fn newest(dir: &Path) -> Option<SystemTime> {
    let mut newest = std::fs::symlink_metadata(dir).ok()?.modified().ok()?;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            newest = newest.max(modified);
        }
    }
    Some(newest)
}

/// Remove session directories under `root` untouched for `max_age`.
/// Returns how many were removed.
pub fn sweep(root: &Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_dir() {
            // Nothing else belongs here; a stray link or file is removed.
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let stale = newest(&path)
            .and_then(|t| now.duration_since(t).ok())
            .is_none_or(|age| age >= max_age);
        if stale && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Daemon start: sweep the real root.
pub fn sweep_stale() {
    if let Some(root) = root() {
        let removed = sweep(&root, STALE_AFTER);
        if removed > 0 {
            tracing::info!(removed, "removed stale codex terminal logs");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("codex-terminal-logs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn logs_are_private_new_files_and_never_follow_links() {
        let root = scratch();
        let dir = root.join("s1");
        ensure_private_dir(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let log = dir.join("t.output");
        create(&log, b"a").unwrap();
        append(&log, b"b").unwrap();
        assert_eq!(std::fs::read(&log).unwrap(), b"ab");
        assert!(create(&log, b"again").is_err(), "create_new");
        #[cfg(unix)]
        {
            let target = root.join("elsewhere");
            std::fs::write(&target, "x").unwrap();
            let link = dir.join("l.output");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(append(&link, b"evil").is_err());
            assert!(create(&link, b"evil").is_err());
            assert_eq!(std::fs::read(&target).unwrap(), b"x");
            // A symlinked session dir is refused.
            let linked = root.join("s2");
            std::os::unix::fs::symlink(root.join("elsewhere-dir"), &linked).unwrap();
            std::fs::create_dir(root.join("elsewhere-dir")).unwrap();
            assert!(ensure_private_dir(&linked).is_err());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sweep_removes_only_stale_session_dirs() {
        let root = scratch();
        ensure_private_dir(&root.join("fresh")).unwrap();
        create(&root.join("fresh").join("t.output"), b"x").unwrap();
        assert_eq!(sweep(&root, Duration::from_secs(3600)), 0);
        assert_eq!(sweep(&root, Duration::ZERO), 1);
        assert!(!root.join("fresh").exists());
        let gone = root.join("gone");
        ensure_private_dir(&gone).unwrap();
        remove_session(&gone);
        assert!(!gone.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
