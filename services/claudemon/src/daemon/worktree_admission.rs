//! Cross-process admission fence shared with the desktop worktree cleaner.
//!
//! A cleaner holds the same per-worktree lock while checking live daemon rows
//! and removing generated artifacts. Overlapping daemon spawns share it until
//! every session has registered, closing the unsafe check-then-spawn window. This is a
//! fail-closed fence: no timeout or file age proves another owner has exited.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

const LOCK_NAME: &str = ".workspacer-maintenance.lock";
const MAX_METADATA_BYTES: u64 = 64 * 1024;

#[derive(Serialize, Deserialize, PartialEq)]
struct Owner {
    pid: u32,
    token: String,
}

pub(crate) struct WorktreeAdmission {
    path: PathBuf,
}

struct FileFence {
    path: PathBuf,
    owner: Owner,
}

struct ActiveAdmission {
    fence: FileFence,
    users: usize,
}

// Concurrent launches in this daemon may share a fence; maintenance and other
// processes still need exclusive ownership of the file. Serialize the final
// release with acquisition so a new launch cannot inherit a disappearing lock.
fn admissions() -> &'static Mutex<HashMap<PathBuf, ActiveAdmission>> {
    static ACTIVE: OnceLock<Mutex<HashMap<PathBuf, ActiveAdmission>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read_metadata(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    File::open(path)?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_METADATA_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "worktree metadata is too large",
        ));
    }
    Ok(text)
}

impl WorktreeAdmission {
    /// Primary checkouts and ordinary directories need no worktree fence.
    /// Resolve the closest Git boundary so a nested cwd locks the same gitdir
    /// as the worktree root, including relative `gitdir:` paths.
    pub(crate) fn acquire(cwd: &str) -> io::Result<Option<Self>> {
        let cwd = fs::canonicalize(cwd)?;
        for ancestor in cwd.ancestors() {
            let marker = ancestor.join(".git");
            let metadata = match fs::symlink_metadata(&marker) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            if metadata.is_dir() {
                return Ok(None);
            }
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported worktree .git entry",
                ));
            }
            let text = read_metadata(&marker)?;
            let target = text
                .trim()
                .strip_prefix("gitdir:")
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid worktree gitdir")
                })?;
            let gitdir = fs::canonicalize(ancestor.join(target))?;
            if !gitdir.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "worktree gitdir is not a directory",
                ));
            }
            let path = gitdir.join(LOCK_NAME);
            let mut active = admissions()
                .lock()
                .map_err(|_| io::Error::other("worktree admission registry poisoned"))?;
            if let Some(entry) = active.get_mut(&path) {
                // Only share our exact, still-owned fence. A matching PID in a
                // foreign/stale file alone is never evidence of ownership.
                let owner: Owner = serde_json::from_str(&read_metadata(&path)?)?;
                if owner != entry.fence.owner {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "worktree admission owner changed",
                    ));
                }
                entry.users += 1;
                return Ok(Some(Self { path }));
            }
            let owner = Owner {
                pid: std::process::id(),
                token: uuid::Uuid::new_v4().to_string(),
            };
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            let fence = FileFence {
                path: path.clone(),
                owner,
            };
            let bytes = serde_json::to_vec(&fence.owner)?;
            // A malformed/partial file on I/O failure is intentionally not
            // stolen. Releasing another owner's fence would be worse than
            // refusing a launch until the failed lock can be inspected.
            file.write_all(&bytes)?;
            file.flush()?;
            active.insert(path.clone(), ActiveAdmission { fence, users: 1 });
            return Ok(Some(Self { path }));
        }
        Ok(None)
    }
}

impl Drop for WorktreeAdmission {
    fn drop(&mut self) {
        // Poisoning must fail closed: retain the on-disk fence if the registry
        // can no longer prove how many launches are using it.
        let Ok(mut active) = admissions().lock() else {
            return;
        };
        if let Some(entry) = active.get_mut(&self.path) {
            entry.users -= 1;
            if entry.users == 0 {
                // Drop the file fence while still holding the registry lock.
                drop(active.remove(&self.path));
            }
        }
    }
}

impl Drop for FileFence {
    fn drop(&mut self) {
        let current = read_metadata(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Owner>(&text).ok());
        if current.as_ref() == Some(&self.owner) {
            if let Err(err) = fs::remove_file(&self.path) {
                tracing::warn!(?err, path = %self.path.display(), "could not release worktree admission lock");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    struct Fixture {
        root: PathBuf,
        repo: PathBuf,
        worktree: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                crate::testtmp::dir().join(format!("worktree-admission-{}", uuid::Uuid::new_v4()));
            let repo = root.join("repo");
            let worktree = root.join("worker");
            fs::create_dir_all(&repo).unwrap();
            fs::write(root.join("empty-config"), "").unwrap();
            let fixture = Self {
                root,
                repo,
                worktree,
            };
            fixture.git(&["init", "--template=", "--initial-branch=main"]);
            fixture.git(&[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ]);
            fixture.git(&[
                "worktree",
                "add",
                "-b",
                "worker",
                fixture.worktree.to_str().unwrap(),
            ]);
            fixture
        }
        fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .args(args)
                .current_dir(&self.repo)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.root.join("empty-config"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn real_linked_worktree_nested_cwd_shares_fence_and_drop_releases_it() {
        let fixture = Fixture::new();
        let nested = fixture.worktree.join("nested/package");
        fs::create_dir_all(&nested).unwrap();
        let guard = WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap())
            .unwrap()
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&guard.path).unwrap()).unwrap();
        assert_eq!(payload["pid"], std::process::id());
        assert!(payload["token"]
            .as_str()
            .is_some_and(|token| uuid::Uuid::parse_str(token).is_ok()));
        let sibling = WorktreeAdmission::acquire(nested.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(guard.path, sibling.path);
        let lock = guard.path.clone();
        drop(guard);
        assert!(lock.exists(), "the sibling launch still needs the fence");
        assert_eq!(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::AlreadyExists,
            "maintenance must remain excluded until the last launch registers"
        );
        drop(sibling);
        assert!(!lock.exists());
        assert!(WorktreeAdmission::acquire(nested.to_str().unwrap())
            .unwrap()
            .is_some());
    }

    #[test]
    fn primary_checkouts_and_non_git_directories_do_not_create_a_fence() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.repo.join("nested")).unwrap();
        assert!(
            WorktreeAdmission::acquire(fixture.repo.join("nested").to_str().unwrap())
                .unwrap()
                .is_none()
        );
        assert!(WorktreeAdmission::acquire(fixture.root.to_str().unwrap())
            .unwrap()
            .is_none());
        assert!(!fixture.repo.join(".git").join(LOCK_NAME).exists());
    }

    #[test]
    fn releasing_a_replaced_lock_never_removes_the_new_owner() {
        let fixture = Fixture::new();
        let guard = WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap())
            .unwrap()
            .unwrap();
        let path = guard.path.clone();
        let successor = r#"{"pid":1234,"token":"different-owner"}"#;
        fs::write(&path, successor).unwrap();
        assert!(WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap()).is_err());
        drop(guard);
        assert_eq!(fs::read_to_string(path).unwrap(), successor);
    }

    #[test]
    fn hundred_overlapping_launches_keep_maintenance_excluded() {
        use std::sync::{Arc, Barrier};
        let fixture = Fixture::new();
        let started = Arc::new(Barrier::new(101));
        let registered = Arc::new(Barrier::new(101));
        // This guard represents the first launch in a burst. Every sibling
        // must join it successfully, rather than return a spurious HTTP 409.
        let first = WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap())
            .unwrap()
            .unwrap();
        let lock = first.path.clone();
        std::thread::scope(|scope| {
            let mut launches = Vec::new();
            for _ in 0..100 {
                let started = started.clone();
                let registered = registered.clone();
                let cwd = &fixture.worktree;
                launches.push(scope.spawn(move || {
                    let admission = WorktreeAdmission::acquire(cwd.to_str().unwrap());
                    // Reach both barriers even on failure so a regression
                    // fails an assertion instead of hanging the suite.
                    started.wait();
                    registered.wait();
                    assert!(admission.unwrap().is_some());
                }));
            }
            started.wait();
            drop(first);
            let maintenance = OpenOptions::new().write(true).create_new(true).open(&lock);
            registered.wait();
            for launch in launches {
                launch.join().unwrap();
            }
            assert_eq!(
                maintenance.err().unwrap().kind(),
                io::ErrorKind::AlreadyExists
            );
        });
        assert!(!lock.exists());
        assert!(!admissions().lock().unwrap().contains_key(&lock));
        // Maintenance can acquire normally once all registrations finish.
        let maintenance = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .unwrap();
        drop(maintenance);
        assert!(WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap()).is_err());
    }

    #[test]
    fn stale_or_malformed_locks_are_not_stolen_by_age_or_pid_guessing() {
        let fixture = Fixture::new();
        let guard = WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap())
            .unwrap()
            .unwrap();
        let path = guard.path.clone();
        drop(guard);
        fs::write(&path, r#"{"pid":4294967295,"token":"dead-or-unknown"}"#).unwrap();
        assert_eq!(
            WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap())
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        fs::write(
            &path,
            serde_json::to_vec(&Owner {
                pid: std::process::id(),
                token: "same-process-but-not-an-active-launch".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap()).is_err());
        fs::write(&path, "partial").unwrap();
        assert_eq!(
            WorktreeAdmission::acquire(fixture.worktree.to_str().unwrap())
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }
}
