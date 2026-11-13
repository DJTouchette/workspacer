//! One-time import of the Electron desktop app's settings and sessions into
//! the native app's own store, so switching apps keeps the session list,
//! config.yaml, profiles, library, jobs and history.
//!
//! The Electron app spreads its state over three places:
//! - its config directory (`%APPDATA%\workspacer`, else
//!   `$XDG_CONFIG_HOME/workspacer` or `~/.config/workspacer`), shared with
//!   Chromium's own caches;
//! - the hub's data directory: the same directory, except on macOS where it is
//!   `~/Library/Application Support/workspacer-hub`;
//! - claudemon's session database (`claudemon::store::default_db_path`).
//!
//! The native store keeps those as `config/`, `hub/` and `state.db` under one
//! directory. Everything is staged in a sibling directory and moved into place
//! only once complete, so a failed or interrupted import leaves no partial
//! store behind and is simply retried on the next launch. Sources are only
//! read: SQLite files are copied with `VACUUM INTO`, a consistent snapshot even
//! while the Electron app is still running.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Whether this launch imports the Electron app's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ElectronImport {
    /// Isolated test or side-by-side instances never import.
    #[default]
    Off,
    /// The default install imports once, into a store that has no data yet.
    IfFresh,
    /// `--import-electron`: import over existing native data, after moving it
    /// into a backup directory beside the store.
    Force,
}

/// Where the Electron app keeps its state on this machine.
#[derive(Clone, Debug)]
pub struct Sources {
    pub config: PathBuf,
    pub hub: PathBuf,
    pub database: PathBuf,
}

impl Sources {
    pub fn detect() -> Option<Self> {
        let dirs = directories::BaseDirs::new()?;
        let config = if cfg!(windows) {
            dirs.config_dir().join("workspacer")
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| dirs.home_dir().join(".config"))
                .join("workspacer")
        };
        let hub = if cfg!(target_os = "macos") {
            dirs.home_dir()
                .join("Library/Application Support/workspacer-hub")
        } else {
            config.clone()
        };
        Some(Self {
            config,
            hub,
            database: claudemon::store::default_db_path(),
        })
    }

    fn present(&self) -> bool {
        self.config.join("config.yaml").is_file() || self.database.is_file()
    }
}

/// What an import did, for the startup notice and the log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub files: usize,
    pub database: bool,
    pub backup: Option<PathBuf>,
}

/// The target already holds native data: a session database or a config.
fn has_native_data(target: &Path) -> bool {
    target.join("state.db").exists() || target.join("config").join("config.yaml").exists()
}

/// Run the import `mode` asks for. `Ok(None)`: nothing to do.
pub fn run(mode: ElectronImport, sources: &Sources, target: &Path) -> Result<Option<Report>> {
    match mode {
        ElectronImport::Off => return Ok(None),
        ElectronImport::IfFresh if has_native_data(target) => return Ok(None),
        _ => {}
    }
    if !sources.present() {
        if mode == ElectronImport::Force {
            bail!(
                "No Electron app data found (looked for {} and {})",
                sources.config.join("config.yaml").display(),
                sources.database.display()
            );
        }
        return Ok(None);
    }
    import(sources, target, mode == ElectronImport::Force).map(Some)
}

fn import(sources: &Sources, target: &Path, replace: bool) -> Result<Report> {
    let parent = target
        .parent()
        .context("Native store has no parent directory")?;
    let name = target.file_name().context("Native store has no name")?;
    let staging = parent.join(format!("{}.electron-import", name.to_string_lossy()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).context("Clear an earlier unfinished import")?;
    }
    let mut report = Report::default();
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(staging.join("config"))?;
        std::fs::create_dir_all(staging.join("hub"))?;
        if sources.config.is_dir() {
            report.files += copy_tree(&sources.config, &staging, Some(&sources.config))?;
        }
        if sources.hub != sources.config && sources.hub.is_dir() {
            report.files += copy_tree(&sources.hub, &staging.join("hub"), None)?;
        }
        if sources.database.is_file() {
            snapshot_sqlite(&sources.database, &staging.join("state.db"))
                .context("Copy the session database")?;
            report.database = true;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }

    std::fs::create_dir_all(target)?;
    if replace {
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let backup = parent.join(format!(
            "{}.before-electron-import-{stamp}",
            name.to_string_lossy()
        ));
        std::fs::create_dir_all(&backup)?;
        for entry in ["config", "hub", "state.db", "state.db-wal", "state.db-shm"] {
            let from = target.join(entry);
            if from.exists() {
                std::fs::rename(&from, backup.join(entry))
                    .with_context(|| format!("Back up {}", from.display()))?;
            }
        }
        report.backup = Some(backup);
    }
    for entry in ["config", "hub", "state.db"] {
        let from = staging.join(entry);
        if !from.exists() {
            continue;
        }
        let to = target.join(entry);
        if to.exists() {
            // IfFresh: a store with only logs or caches. Keep anything the
            // native app already wrote that the import did not bring.
            merge_into(&from, &to)?;
        } else {
            std::fs::rename(&from, &to)
                .with_context(|| format!("Move imported {entry} into place"))?;
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::write(
        target.join("imported-from-electron.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "config": sources.config,
            "hub": sources.hub,
            "database": sources.database,
            "files": report.files,
            "backup": report.backup,
        }))?,
    )?;
    Ok(report)
}

/// Chromium's profile data and Electron-only files never leave the Electron
/// directory. Port, lock and SQLite journal files belong to a running owner.
fn skipped(name: &str) -> bool {
    const ELECTRON_ONLY: &[&str] = &[
        "Cache",
        "Code Cache",
        "GPUCache",
        "DawnGraphiteCache",
        "DawnWebGPUCache",
        "GrShaderCache",
        "ShaderCache",
        "IndexedDB",
        "Local Storage",
        "Session Storage",
        "Network",
        "Partitions",
        "Service Worker",
        "Shared Dictionary",
        "SharedStorage",
        "SharedStorage-wal",
        "VideoDecodeStats",
        "WebStorage",
        "blob_storage",
        "shared_proto_db",
        "Crashpad",
        "Local State",
        "Preferences",
        "TransportSecurity",
        "Trust Tokens",
        "Trust Tokens-journal",
        "DIPS",
        "DIPS-journal",
        "Conversions",
        "Conversions-journal",
        ".updaterId",
        ".font-cache.json",
        "logs",
        "workspacer.db",
        "hub-port",
        "mcp-port",
    ];
    ELECTRON_ONLY.contains(&name)
        || name.ends_with(".lock")
        || name.ends_with("-wal")
        || name.ends_with("-shm")
        || name.ends_with("-journal")
        || name.starts_with("workspacer.db")
        || name.contains(".bak")
        || name.starts_with(".tmp")
}

/// The hub's own state files, which the native store keeps under `hub/`.
fn hub_file(name: &str) -> bool {
    matches!(
        name,
        "agent-launches.json"
            | "jobs.json"
            | "jobs-history.json"
            | "session-archive.json"
            | "session-open.json"
            | "session-seen.json"
    )
}

/// Copy `from` into `to`. With `split` (the Electron config root), hub state
/// files go to `to/hub` and everything else to `to/config`.
fn copy_tree(from: &Path, to: &Path, split: Option<&Path>) -> Result<usize> {
    let mut copied = 0;
    for entry in std::fs::read_dir(from).with_context(|| format!("Read {}", from.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if skipped(&name) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let destination = match split {
            Some(_) if hub_file(&name) => to.join("hub").join(&*name),
            Some(_) => to.join("config").join(&*name),
            None => to.join(&*name),
        };
        if kind.is_dir() {
            std::fs::create_dir_all(&destination)?;
            copied += copy_tree(&entry.path(), &destination, None)?;
        } else if is_sqlite(&name) && snapshot_sqlite(&entry.path(), &destination).is_ok() {
            copied += 1;
        } else {
            // Not SQLite after all (or unreadable as such): a plain copy.
            let _ = std::fs::remove_file(&destination);
            std::fs::copy(entry.path(), &destination)
                .with_context(|| format!("Copy {}", entry.path().display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn is_sqlite(name: &str) -> bool {
    name.ends_with(".sqlite") || name.ends_with(".db")
}

/// A consistent copy of a live SQLite database (WAL included).
fn snapshot_sqlite(from: &Path, to: &Path) -> Result<()> {
    let source = rusqlite::Connection::open_with_flags(
        from,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    source.busy_timeout(std::time::Duration::from_secs(30))?;
    let destination = to.to_str().context("Database path is not UTF-8")?;
    source.execute("VACUUM INTO ?1", [destination])?;
    Ok(())
}

/// Move everything in `from` into `to`, keeping files `to` already has.
fn merge_into(from: &Path, to: &Path) -> Result<()> {
    if from.is_file() {
        return Ok(()); // An existing state.db is never replaced here.
    }
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let destination = to.join(entry.file_name());
        if entry.file_type()?.is_dir() && destination.is_dir() {
            merge_into(&entry.path(), &destination)?;
        } else if !destination.exists() {
            std::fs::rename(entry.path(), &destination)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wks-electron-import-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A WAL database. Electron-side sources stay open, as under a running
    /// Electron app; the native store is closed, as before its backend starts.
    fn sqlite(path: &Path, value: &str) {
        std::mem::forget(closed_sqlite(path, value));
    }

    fn closed_sqlite(path: &Path, value: &str) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(v TEXT);")
            .unwrap();
        db.execute("INSERT INTO t VALUES (?1)", [value]).unwrap();
        db
    }

    fn read(path: &Path) -> String {
        rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT v FROM t", [], |r| r.get(0))
            .unwrap()
    }

    fn electron(root: &Path) -> Sources {
        let config = root.join("electron");
        std::fs::create_dir_all(config.join("Cache")).unwrap();
        std::fs::create_dir_all(config.join("library")).unwrap();
        std::fs::write(config.join("config.yaml"), "theme: dark\n").unwrap();
        std::fs::write(config.join("Cache/data_0"), "chromium").unwrap();
        std::fs::write(config.join("Local State"), "{}").unwrap();
        std::fs::write(config.join("hub-port"), "7895\n").unwrap();
        std::fs::write(config.join("config.yaml.bak-1"), "old").unwrap();
        std::fs::write(config.join("library/note.md"), "note").unwrap();
        std::fs::write(config.join("jobs.json"), "[]").unwrap();
        sqlite(&config.join("intent-workspaces.sqlite"), "intent");
        let database = root.join("claudemon/state.db");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        sqlite(&database, "sessions");
        Sources {
            hub: config.clone(),
            config,
            database,
        }
    }

    #[test]
    fn a_fresh_store_receives_config_hub_state_and_sessions() {
        let root = temp("fresh");
        let sources = electron(&root);
        let target = root.join("native");
        let report = run(ElectronImport::IfFresh, &sources, &target)
            .unwrap()
            .unwrap();
        assert!(report.database);
        assert_eq!(
            std::fs::read_to_string(target.join("config/config.yaml")).unwrap(),
            "theme: dark\n"
        );
        assert!(target.join("config/library/note.md").is_file());
        assert!(target.join("hub/jobs.json").is_file());
        assert_eq!(read(&target.join("state.db")), "sessions");
        assert_eq!(
            read(&target.join("config/intent-workspaces.sqlite")),
            "intent"
        );
        for skipped in [
            "config/Cache",
            "config/Local State",
            "config/hub-port",
            "config/config.yaml.bak-1",
        ] {
            assert!(!target.join(skipped).exists(), "{skipped} was imported");
        }
        assert!(target.join("imported-from-electron.json").is_file());
        assert!(!root.join("native.electron-import").exists());
        // Sources are untouched.
        assert!(sources.config.join("Cache/data_0").is_file());
        // Once imported, a later launch leaves the store alone.
        std::fs::write(target.join("config/config.yaml"), "theme: light\n").unwrap();
        assert!(
            run(ElectronImport::IfFresh, &sources, &target)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            std::fs::read_to_string(target.join("config/config.yaml")).unwrap(),
            "theme: light\n"
        );
    }

    #[test]
    fn force_backs_up_existing_native_data_first() {
        let root = temp("force");
        let sources = electron(&root);
        let target = root.join("native");
        std::fs::create_dir_all(target.join("config")).unwrap();
        std::fs::write(target.join("config/config.yaml"), "native: true\n").unwrap();
        drop(closed_sqlite(&target.join("state.db"), "native sessions"));
        let report = run(ElectronImport::Force, &sources, &target)
            .unwrap()
            .unwrap();
        let backup = report.backup.unwrap();
        assert_eq!(
            std::fs::read_to_string(backup.join("config/config.yaml")).unwrap(),
            "native: true\n"
        );
        assert_eq!(read(&backup.join("state.db")), "native sessions");
        assert_eq!(read(&target.join("state.db")), "sessions");
        assert_eq!(
            std::fs::read_to_string(target.join("config/config.yaml")).unwrap(),
            "theme: dark\n"
        );
    }

    #[test]
    fn nothing_to_import_is_not_an_error_unless_forced() {
        let root = temp("none");
        let sources = Sources {
            config: root.join("missing"),
            hub: root.join("missing"),
            database: root.join("missing/state.db"),
        };
        let target = root.join("native");
        assert!(
            run(ElectronImport::IfFresh, &sources, &target)
                .unwrap()
                .is_none()
        );
        assert!(run(ElectronImport::Force, &sources, &target).is_err());
        assert!(!target.exists());
    }

    #[test]
    fn electron_only_and_owner_files_are_skipped() {
        for name in [
            "Cache",
            "Local State",
            "workspacer.db",
            "workspacer.db-wal",
            "x.lock",
            "hub-port",
            ".tmpABC",
        ] {
            assert!(skipped(name), "{name}");
        }
        for name in [
            "config.yaml",
            "library",
            "tokens.json",
            "native-settings.json",
            "sessions",
        ] {
            assert!(!skipped(name), "{name}");
        }
    }
}
