//! Per-project scratch: personal working files kept outside every checkout.
//!
//! Agent worktrees are removed with `git worktree remove`, which deletes
//! ignored files with the tree, so scratch cannot live inside the repository.
//! It lives at `~/.workspacer/scratch/<key>/files`, where `<key>` names the
//! project's *main* checkout — every linked worktree of a repository resolves
//! to the same scratch. This module is the single derivation: claudemon uses it
//! to set [`ENV`] on spawned agents and the hub's `scratch.*` service uses it to
//! find the same directory. See `docs/features/scratch.md`.
//!
//! The derivation is pure filesystem (no `git` subprocess) because it runs on
//! every spawn.

use std::path::{Path, PathBuf};

/// The env var spawned agents receive, pointing at the project's `files/` dir.
pub const ENV: &str = "WORKSPACER_SCRATCH";

/// Resolve the project root a `cwd` belongs to: the main checkout for a linked
/// worktree, the repository root for a primary checkout, or the canonical cwd
/// itself outside git. `None` only when the cwd cannot be canonicalized.
pub fn project_root(cwd: &Path) -> Option<PathBuf> {
    resolve(cwd).map(|(root, _)| root)
}

/// [`project_root`] plus whether the cwd is inside a git repository.
fn resolve(cwd: &Path) -> Option<(PathBuf, bool)> {
    let cwd = std::fs::canonicalize(cwd).ok()?;
    for dir in cwd.ancestors() {
        let marker = dir.join(".git");
        let Ok(meta) = std::fs::symlink_metadata(&marker) else {
            continue;
        };
        if meta.is_file() {
            let root = linked_main_checkout(dir, &marker).unwrap_or_else(|| dir.to_path_buf());
            return Some((root, true));
        }
        // A `.git` directory marks the root. A symlinked `.git` is not a layout
        // git itself produces; the directory holding it is treated as the root
        // rather than following the link.
        return Some((dir.to_path_buf(), true));
    }
    Some((cwd, false))
}

/// For a `.git` *file*: follow `gitdir:` and then `commondir`. A gitdir without
/// a `commondir` (a submodule) is its own project, so the caller falls back to
/// the directory holding the `.git` file.
fn linked_main_checkout(dir: &Path, marker: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(marker).ok()?;
    let gitdir = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    let gitdir = dir.join(gitdir);
    let common = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let common = std::fs::canonicalize(gitdir.join(common.trim())).ok()?;
    if common.file_name().is_some_and(|name| name == ".git") {
        let main = common.parent()?.to_path_buf();
        main.is_dir().then_some(main)
    } else {
        // Bare repository: the common dir itself names the project.
        Some(common)
    }
}

/// Directory-name key for a project root: a readable, sanitized basename plus a
/// stable 64-bit FNV-1a hash of the full path, so same-named repositories in
/// different places never share scratch.
pub fn key(root: &Path) -> String {
    let base: String = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let base = base.trim_matches(|c| c == '.' || c == '-');
    let base = if base.is_empty() { "project" } else { base };
    format!("{base}-{:016x}", fnv1a64(root.to_string_lossy().as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// `<home>/.workspacer/scratch` — the parent of every project's scratch.
pub fn base(home: &Path) -> PathBuf {
    home.join(".workspacer").join("scratch")
}

/// `<home>/.workspacer/scratch/<key>` for a project root.
pub fn project_dir(home: &Path, root: &Path) -> PathBuf {
    base(home).join(key(root))
}

/// The user's home, as every other `~/.workspacer` path in claudemon finds it.
pub fn home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

/// Create (if needed) the scratch for the project owning `cwd` and return its
/// `files/` directory. Directories are private (0700) on unix; `project.json`
/// records the resolved root so a human browsing `~/.workspacer/scratch` can
/// tell projects apart. An existing `project.json` is never rewritten.
pub fn ensure(home: &Path, cwd: &Path) -> std::io::Result<PathBuf> {
    let root = project_root(cwd).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("cannot resolve a project for {}", cwd.display()),
        )
    })?;
    let project = project_dir(home, &root);
    let files = project.join("files");
    create_private_dir_all(&files)?;
    let record = project.join("project.json");
    if !record.exists() {
        let body = serde_json::json!({"version": 1, "root": root});
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&record)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(serde_json::to_string_pretty(&body)?.as_bytes())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(files)
}

fn create_private_dir_all(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Spawn-time injection: set [`ENV`] for a child unless the caller already
/// chose a value. Best-effort — a spawn never fails because scratch could not
/// be prepared; the agent simply does not get the variable.
///
/// Only a cwd inside a git repository gets one, and never a project under the
/// system temp dir: agents (and test suites) launched in throwaway directories
/// would otherwise leave one empty scratch per temp dir behind. `scratch.save`
/// through the hub still works for any directory.
pub fn inject_env(env: &mut std::collections::HashMap<String, String>, cwd: &str) {
    if env.contains_key(ENV) || cwd.is_empty() {
        return;
    }
    let Some(home) = home() else {
        return;
    };
    let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
    if project_root(Path::new(cwd)).is_some_and(|root| root.starts_with(&temp)) {
        return;
    }
    inject_env_with_home(env, cwd, &home);
}

/// [`inject_env`] with an explicit home and without the temp-dir exclusion,
/// for tests.
pub fn inject_env_with_home(
    env: &mut std::collections::HashMap<String, String>,
    cwd: &str,
    home: &Path,
) {
    if env.contains_key(ENV) || cwd.is_empty() {
        return;
    }
    if !resolve(Path::new(cwd)).is_some_and(|(_, git)| git) {
        return;
    }
    match ensure(home, Path::new(cwd)) {
        Ok(files) => {
            env.insert(ENV.into(), files.to_string_lossy().into_owned());
        }
        Err(error) => eprintln!("scratch: not preparing {ENV} for {cwd}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A unique directory under the crate's single test-debris dir, removed on
    /// drop (see `crate::testtmp` for why tests do not litter /tmp directly).
    struct Tmp(PathBuf);
    impl Tmp {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tmp() -> Tmp {
        let dir = crate::testtmp::dir().join(format!("scratch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(
            status.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn repo(at: &Path) {
        std::fs::create_dir_all(at).unwrap();
        git(at, &["init", "-q", "-b", "main"]);
        git(
            at,
            &[
                "-c",
                "user.email=t@example.com",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
    }

    #[test]
    fn a_subdirectory_resolves_to_its_repository_root() {
        let tmp = tmp();
        let main = tmp.path().join("shop");
        repo(&main);
        std::fs::create_dir_all(main.join("apps/web")).unwrap();
        let root = std::fs::canonicalize(&main).unwrap();
        assert_eq!(project_root(&main.join("apps/web")), Some(root.clone()));
        assert_eq!(project_root(&main), Some(root));
    }

    #[test]
    fn every_linked_worktree_shares_the_main_checkouts_scratch() {
        let tmp = tmp();
        let main = tmp.path().join("shop");
        repo(&main);
        let wt = tmp.path().join("worktrees/shop/fix-cart");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wks/fix",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::create_dir_all(wt.join("src")).unwrap();
        let root = std::fs::canonicalize(&main).unwrap();
        assert_eq!(project_root(&wt), Some(root.clone()));
        assert_eq!(project_root(&wt.join("src")), Some(root.clone()));
        let home = tmp.path().join("home");
        assert_eq!(
            ensure(&home, &wt).unwrap(),
            ensure(&home, &main).unwrap(),
            "worktree and main checkout use one scratch"
        );
    }

    #[test]
    fn outside_git_the_directory_is_its_own_project() {
        let tmp = tmp();
        let plain = tmp.path().join("notes");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(
            project_root(&plain),
            Some(std::fs::canonicalize(&plain).unwrap())
        );
        assert_eq!(project_root(&tmp.path().join("missing")), None);
    }

    #[test]
    fn a_submodule_style_gitdir_without_commondir_is_its_own_project() {
        let tmp = tmp();
        let sub = tmp.path().join("vendor/lib");
        std::fs::create_dir_all(&sub).unwrap();
        let modules = tmp.path().join(".git/modules/lib");
        std::fs::create_dir_all(&modules).unwrap();
        std::fs::write(sub.join(".git"), "gitdir: ../../.git/modules/lib\n").unwrap();
        assert_eq!(
            project_root(&sub),
            Some(std::fs::canonicalize(&sub).unwrap())
        );
    }

    #[test]
    fn keys_are_readable_stable_and_distinguish_same_named_projects() {
        let a = key(Path::new("/home/me/work/shop"));
        let b = key(Path::new("/home/me/home/shop"));
        assert!(
            a.starts_with("shop-") && a.len() == "shop-".len() + 16,
            "{a}"
        );
        assert_ne!(a, b);
        assert_eq!(a, key(Path::new("/home/me/work/shop")));
        assert!(key(Path::new("/")).starts_with("project-"));
        let odd = key(Path::new("/x/My Repo (old)"));
        assert!(odd.starts_with("My-Repo--old-"), "{odd}");
    }

    #[test]
    fn ensure_records_the_root_once_and_injection_never_overrides_the_caller() {
        let tmp = tmp();
        let main = tmp.path().join("shop");
        repo(&main);
        let home = tmp.path().join("home");
        let files = ensure(&home, &main).unwrap();
        assert!(files.is_dir() && files.ends_with("files"));
        let record: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(files.parent().unwrap().join("project.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            record["root"],
            serde_json::json!(std::fs::canonicalize(&main).unwrap())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&files).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }

        let mut env = std::collections::HashMap::new();
        inject_env_with_home(&mut env, main.to_str().unwrap(), &home);
        assert_eq!(env[ENV], files.to_string_lossy());

        let mut chosen = std::collections::HashMap::from([(ENV.to_string(), "/mine".to_string())]);
        inject_env_with_home(&mut chosen, main.to_str().unwrap(), &home);
        assert_eq!(chosen[ENV], "/mine");

        let mut missing = std::collections::HashMap::new();
        inject_env_with_home(
            &mut missing,
            tmp.path().join("gone").to_str().unwrap(),
            &home,
        );
        assert!(
            !missing.contains_key(ENV),
            "an unresolvable cwd skips, never fails"
        );

        let plain = tmp.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let mut outside = std::collections::HashMap::new();
        inject_env_with_home(&mut outside, plain.to_str().unwrap(), &home);
        assert!(
            !outside.contains_key(ENV),
            "a directory outside git gets no scratch at spawn"
        );

        let mut temp = std::collections::HashMap::new();
        inject_env(&mut temp, main.to_str().unwrap());
        assert!(
            !temp.contains_key(ENV),
            "a repository under the temp dir is skipped"
        );
    }
}
