//! Cooperative ownership for Rust Backend hosts sharing one persistent DB.
//! The standalone claudemon CLI does not participate in this sidecar protocol.
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub(super) struct DatabaseLease {
    pub database: PathBuf,
    _file: File,
}
impl DatabaseLease {
    pub fn take(path: &Path) -> Result<Option<Self>> {
        // SQLite's explicit in-memory database is private to each process.
        if path == Path::new(":memory:") {
            return Ok(None);
        }
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let name = path
            .file_name()
            .context("backend database path must name a file")?;
        let parent = path
            .parent()
            .context("backend database path has no parent")?;
        std::fs::create_dir_all(parent)?;
        let database = match std::fs::canonicalize(&path) {
            Ok(path) => {
                anyhow::ensure!(
                    std::fs::metadata(&path)?.is_file(),
                    "backend database must be a regular file"
                );
                path
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                anyhow::ensure!(
                    std::fs::symlink_metadata(&path)
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
                    "cannot establish ownership for an unresolved database link"
                );
                std::fs::canonicalize(parent)?.join(name)
            }
            Err(error) => return Err(error.into()),
        };
        let mut name = database
            .file_name()
            .context("backend database path must name a file")?
            .to_os_string();
        name.push(".rust-owner.lock");
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(database.with_file_name(name))
            .context("opening Rust backend ownership lock")?;
        file.try_lock_exclusive().with_context(||format!("database ownership unavailable for {}; stop its existing Rust backend owner (or a process with unconfirmed cleanup) before reusing it",database.display()))?;
        // Never unlink this stable lock inode: other owners may have opened it.
        Ok(Some(Self {
            database,
            _file: file,
        }))
    }
}
