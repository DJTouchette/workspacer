//! Reclaim recognized ignored build/dependency artifacts from stopped agent
//! worktrees. Never removes a worktree, branch, source file, tracked path,
//! arbitrary ignored directory, or symlink target.
//!
//! Every check fails closed: anything unexpected skips the worktree or the
//! artifact and is reported, and unknown daemon state is never an empty fleet.
//! The whole inspection of one worktree runs under the same per-Git-directory
//! maintenance file that daemon spawn admission and worktree creation take, so
//! a launch cannot register between the liveness check and the removal.
//!
//! The filesystem walks here are synchronous. Run a pass on a thread that may
//! block (the scheduler gives each pass its own), not on a shared runtime.
use super::super::paths;
use anyhow::{Result, anyhow, bail};
use claudemon::daemon::WorktreeMaintenance;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::Metadata,
    future::Future,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};

/// Written into a linked worktree's Git administration directory at allocation.
pub const WORKTREE_ALLOCATION: &str = "workspacer-allocation.json";
const MAX_METADATA_BYTES: u64 = 64 * 1024;
const REFERENCE_SCAN_DIRECTORIES: usize = 50_000;
const REFERENCE_SCAN_BUDGET: Duration = Duration::from_secs(30);
const STATE_TIMEOUT: Duration = Duration::from_secs(10);
/// Directory names that may be generated output; [`generated`] still requires
/// matching project metadata beside them.
const ARTIFACT_NAMES: [&str; 12] = [
    "node_modules",
    "bin",
    "obj",
    "target",
    "build",
    "dist",
    ".next",
    ".nuxt",
    ".turbo",
    ".gradle",
    "__pycache__",
    ".venv",
];

/// The daemon's view of the fleet, read under the maintenance lease.
#[derive(Debug, Clone, Default)]
pub struct CleanupState {
    /// The daemon takes the shared maintenance file before registering a spawn.
    pub maintenance_supported: bool,
    /// Session rows; each must be an object with a string `mode`.
    pub sessions: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Planned,
    Removed,
    Skipped,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Artifact {
    pub worktree: String,
    pub path: String,
    pub bytes: u64,
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Skipped {
    pub worktree: String,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportError {
    pub path: String,
    pub message: String,
}
/// Reported bytes are an estimate (symlink targets and multiply-linked files
/// are excluded), not a measured change in free disk space.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub root: String,
    pub apply: bool,
    pub scanned_worktrees: usize,
    pub removed_bytes: u64,
    pub reclaimable_bytes: u64,
    pub artifacts: Vec<Artifact>,
    pub skipped: Vec<Skipped>,
    pub errors: Vec<ReportError>,
}
impl CleanupReport {
    pub fn is_quiet(&self) -> bool {
        self.artifacts.is_empty() && self.errors.is_empty() && self.skipped.is_empty()
    }
}

pub struct Options<'a> {
    pub root: &'a Path,
    /// false is a dry run: nothing is removed.
    pub apply: bool,
    /// Minimum age of the allocation, of an associated stopped session and of
    /// the newest write inside each artifact.
    pub min_age: Duration,
    /// Set on shutdown: no further worktree is opened and nothing is removed.
    pub stop: Option<&'a AtomicBool>,
}

/// Checked before each worktree, each directory of a walk and each removal,
/// so a shutdown releases the maintenance lease promptly.
fn proceed(options: &Options<'_>) -> Result<()> {
    if options
        .stop
        .is_some_and(|stop| stop.load(Ordering::Acquire))
    {
        bail!("Artifact cleanup stopped");
    }
    Ok(())
}

/// Read the fleet from a claudemon API. The maintenance marker is a response
/// header so the health payload contract stays unchanged.
pub async fn load_worktree_cleanup_state(
    client: &reqwest::Client,
    daemon_url: &str,
) -> Result<CleanupState> {
    let daemon_url = daemon_url.trim_end_matches('/');
    let (health, sessions) = tokio::try_join!(
        client
            .get(format!("{daemon_url}/health"))
            .timeout(STATE_TIMEOUT)
            .send(),
        client
            .get(format!(
                "{daemon_url}/sessions?state_only=true&include_archived=true&include_empty=true"
            ))
            .timeout(STATE_TIMEOUT)
            .send(),
    )?;
    if !health.status().is_success() || !sessions.status().is_success() {
        bail!("Daemon state unavailable; cleanup skipped");
    }
    let maintenance_supported = health
        .headers()
        .get("X-Workspacer-Maintenance")
        .is_some_and(|value| value == "1");
    let sessions = tokio::time::timeout(STATE_TIMEOUT, sessions.json::<Value>())
        .await
        .map_err(|_| anyhow!("Daemon state unavailable; cleanup skipped"))??;
    Ok(CleanupState {
        maintenance_supported,
        sessions: valid_sessions(sessions)?,
    })
}

fn valid_sessions(sessions: Value) -> Result<Vec<Value>> {
    match sessions {
        Value::Array(rows) if rows.iter().all(|row| row["mode"].is_string()) => Ok(rows),
        _ => bail!("Invalid daemon session state; cleanup skipped"),
    }
}

async fn git(cwd: &Path, args: &[&str]) -> (bool, String) {
    match super::git(cwd, args).await {
        Ok(output) => (output.ok, output.stdout),
        Err(_) => (false, String::new()),
    }
}

/// Lexical absolute form for a path that cannot be resolved on disk.
fn lexical(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut normal = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    normal
}
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| lexical(path))
}
/// `candidate` is `root` or inside it. Both must already be canonical.
fn contains(root: &Path, candidate: &Path) -> bool {
    paths::contained(candidate, root)
}
fn overlaps(a: &Path, b: &Path) -> bool {
    contains(a, b) || contains(b, a)
}

/// Device and inode. Without them a replaced directory is indistinguishable
/// from the inspected one, so platforms that lack them never clean.
fn identity(metadata: &Metadata) -> Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        bail!("Directory identity is unavailable on this platform; cleanup skipped")
    }
}
/// Allocated bytes of a file only this directory entry owns. Hardlinks may
/// belong to a shared package store; do not claim their bytes will be freed.
fn owned_bytes(metadata: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() <= 1 {
            metadata.blocks().saturating_mul(512)
        } else {
            0
        }
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

/// `Ok(false)` only when the path is definitely absent.
fn exists_no_follow(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn bounded_text(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_METADATA_BYTES {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "worktree metadata is too large",
        ));
    }
    Ok(text)
}

/// Same ancestor/file rule as claudemon's worktree admission guard: the
/// closest `.git` entry decides, and only a `gitdir:` file names a linked
/// worktree. Returns the worktree root and its Git administration directory.
pub fn linked_worktree(cwd: &Path) -> Result<Option<(PathBuf, PathBuf)>> {
    let start = canonical(cwd);
    for dir in start.ancestors() {
        let marker = dir.join(".git");
        let metadata = match std::fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Ok(None);
        }
        let text = bounded_text(&marker)?;
        let Some(target) = text
            .strip_prefix("gitdir:")
            .map(str::trim)
            .filter(|target| !target.is_empty() && !target.contains(['\n', '\r']))
        else {
            return Ok(None);
        };
        let git_dir = std::fs::canonicalize(dir.join(target))?;
        return Ok(std::fs::metadata(&git_dir)?
            .is_dir()
            .then(|| (dir.to_path_buf(), git_dir)));
    }
    Ok(None)
}

/// A recognized name counts only beside the project metadata that produces it.
fn generated(name: &str, files: &BTreeSet<OsString>) -> bool {
    let has = |file: &str| files.contains(std::ffi::OsStr::new(file));
    let node = has("package.json");
    let dotnet = files.iter().any(|file| {
        file.to_str().is_some_and(|file| {
            [".csproj", ".fsproj", ".vbproj"]
                .iter()
                .any(|extension| file.ends_with(extension))
        })
    });
    let gradle = has("build.gradle") || has("build.gradle.kts");
    match name {
        "node_modules" | "dist" | ".next" | ".nuxt" | ".turbo" => node,
        "obj" => dotnet,
        "bin" => dotnet || has("go.mod"),
        "target" => has("Cargo.toml") || has("pom.xml"),
        "build" => node || gradle || has("CMakeLists.txt"),
        ".gradle" => gradle,
        ".venv" => has("pyproject.toml") || has("requirements.txt"),
        _ => name == "__pycache__",
    }
}

/// Directory entries with their own (never followed) types, in name order.
fn entries(dir: &Path) -> Result<Vec<(OsString, std::fs::FileType)>> {
    let mut listed = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        listed.push((entry.file_name(), entry.file_type()?));
    }
    listed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(listed)
}

/// Bounded-to-source candidate walk. Full reference discovery is separate.
fn scan(cwd: &Path, options: &Options<'_>) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    let mut pending = vec![cwd.to_path_buf()];
    while let Some(dir) = pending.pop() {
        proceed(options)?;
        let listed = entries(&dir)?;
        let files: BTreeSet<OsString> = listed
            .iter()
            .filter(|(_, kind)| kind.is_file())
            .map(|(name, _)| name.clone())
            .collect();
        let mut children = Vec::new();
        for (name, kind) in &listed {
            if kind.is_symlink() || !kind.is_dir() {
                continue;
            }
            let full = dir.join(name);
            let text = name.to_str();
            if let Some(known) = text.filter(|text| ARTIFACT_NAMES.contains(text)) {
                if generated(known, &files) {
                    candidates.push(full);
                }
                continue;
            }
            if name.as_encoded_bytes().first() == Some(&b'.') {
                continue;
            }
            // A nested checkout belongs to its own lifecycle.
            if exists_no_follow(&full.join(".git"))? {
                continue;
            }
            children.push(full);
        }
        // Depth-first in name order: the last pushed is visited first.
        pending.extend(children.into_iter().rev());
    }
    Ok(candidates)
}

/// Estimated reclaimable bytes and the newest write anywhere inside `dir`.
/// Symlinks are neither followed nor counted.
fn measure(dir: &Path, options: &Options<'_>) -> Result<(u64, SystemTime)> {
    let mut bytes = 0u64;
    let mut modified = std::fs::symlink_metadata(dir)?.modified()?;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        proceed(options)?;
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            if entry.file_name() == ".git" {
                bail!("Contains a nested Git checkout");
            }
            let full = entry.path();
            let metadata = std::fs::symlink_metadata(&full)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            modified = modified.max(metadata.modified()?);
            if metadata.is_dir() {
                pending.push(full);
            } else {
                bytes = bytes.saturating_add(owned_bytes(&metadata));
            }
        }
    }
    Ok((bytes, modified))
}

struct Reference {
    source: PathBuf,
    target: PathBuf,
}

/// Every symlink in every checkout Git has registered for this repository.
/// Git's registry includes the primary checkout and siblings under OTHER
/// configured roots; restricting discovery to this sweep's root misses them.
/// Links are inspected, never traversed; an incomplete scan is an error.
async fn shared_references(cwd: &Path, options: &Options<'_>) -> Result<Vec<Reference>> {
    let (ok, output) = git(cwd, &["worktree", "list", "--porcelain", "-z"]).await;
    if !ok {
        bail!("Cannot inspect registered worktree references");
    }
    if output.contains('\u{fffd}') {
        bail!("A registered worktree path is not Unicode; cleanup skipped");
    }
    let roots: Vec<PathBuf> = output
        .split('\0')
        .filter_map(|part| part.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect();
    if roots.is_empty() {
        bail!("Invalid worktree registry");
    }
    let mut references = Vec::new();
    let mut directories = 0usize;
    let deadline = Instant::now() + REFERENCE_SCAN_BUDGET;
    for root in roots {
        match std::fs::symlink_metadata(&root) {
            // A prunable/missing checkout holds no links.
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
                bail!("Registered worktree path changed")
            }
            Ok(_) => {}
        }
        let mut pending = vec![root];
        while let Some(dir) = pending.pop() {
            proceed(options)?;
            directories += 1;
            if directories > REFERENCE_SCAN_DIRECTORIES || Instant::now() > deadline {
                bail!("Shared-reference scan exceeded its budget; cleanup skipped");
            }
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                if entry.file_name() == ".git" {
                    continue;
                }
                let kind = entry.file_type()?;
                let full = entry.path();
                if kind.is_symlink() {
                    match std::fs::canonicalize(&full) {
                        Ok(target) => references.push(Reference {
                            source: full,
                            target,
                        }),
                        // A dangling link references nothing.
                        Err(error) if error.kind() == ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                } else if kind.is_dir() {
                    pending.push(full);
                }
            }
        }
    }
    Ok(references)
}

/// Whether `time` is at least `min_age` in the past. A timestamp from the
/// future is never old enough.
fn aged(time: SystemTime, min_age: Duration) -> bool {
    SystemTime::now()
        .duration_since(time)
        .is_ok_and(|age| age >= min_age)
}
fn timestamp(value: &Value) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(SystemTime::from)
}

/// Why the fleet still protects this worktree, if it does. Idle is live.
fn inactivity_reason(cwd: &Path, state: &CleanupState, min_age: Duration) -> Option<&'static str> {
    const UNVERIFIABLE: &str = "A live session has no verifiable working directory";
    for session in &state.sessions {
        let live = session["mode"] != "stopped";
        let raw: Vec<&str> = [
            &session["cwd"],
            &session["live_cwd"],
            &session["liveCwd"],
            &session["status_line"]["cwd"],
            &session["status_line"]["workspace"]["current_dir"],
            &session["status_line"]["workspace"]["project_dir"],
        ]
        .into_iter()
        .filter_map(Value::as_str)
        .filter(|path| !path.is_empty())
        .collect();
        if live && (raw.is_empty() || raw.iter().any(|path| !Path::new(path).is_absolute())) {
            return Some(UNVERIFIABLE);
        }
        let mut uses = false;
        for path in raw {
            let resolved = match paths::canonicalize(Path::new(path)) {
                Ok(resolved) => resolved,
                Err(_) if live => return Some(UNVERIFIABLE),
                Err(_) => lexical(Path::new(path)),
            };
            uses |= overlaps(cwd, &resolved);
        }
        if !uses {
            continue;
        }
        if live {
            return Some("A live session uses this worktree");
        }
        if !timestamp(&session["updated_at"]).is_some_and(|stopped| aged(stopped, min_age)) {
            return Some("Recently stopped session; waiting for cleanup cooldown");
        }
    }
    None
}

struct Tree {
    cwd: PathBuf,
    git_dir: PathBuf,
    created: SystemTime,
    identity: (u64, u64),
}

/// `Ok(None)`: not a managed linked worktree, silently not ours.
/// `Err`: looks managed but something is off; reported and never cleaned.
async fn managed_worktree(cwd: &Path, root: &Path) -> Result<Option<Tree>> {
    let Some((linked_cwd, git_dir)) = linked_worktree(cwd)? else {
        return Ok(None);
    };
    if linked_cwd != cwd || !git(cwd, &["rev-parse", "--is-inside-work-tree"]).await.0 {
        return Ok(None);
    }
    let (ok, common) = git(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await;
    if !ok {
        return Ok(None);
    }
    if common.contains('\u{fffd}') {
        bail!("Git common directory is not Unicode; not cleaning");
    }
    // A checkout whose own administration directory is the repository is a
    // primary checkout, however its `.git` entry is spelled.
    if canonical(Path::new(common.trim())) == git_dir {
        return Ok(None);
    }
    let metadata = std::fs::symlink_metadata(cwd)?;
    let identity = identity(&metadata)?;
    let mut created = metadata.modified()?;
    match bounded_text(&git_dir.join(WORKTREE_ALLOCATION)) {
        Ok(marker) => {
            let record: Value = serde_json::from_str(&marker)?;
            let recorded = timestamp(&record["createdAt"]);
            if record["version"].as_u64() != Some(1)
                || record["cwd"].as_str().map(Path::new) != Some(cwd)
                || record["root"].as_str().map(Path::new) != Some(root)
                || record["dev"].as_u64() != Some(identity.0)
                || record["ino"].as_u64() != Some(identity.1)
                || recorded.is_none()
            {
                bail!("Allocation identity changed; not cleaning");
            }
            created = recorded.expect("checked above");
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            // Allocations older than the record are recognized by branch.
            let (ok, branch) = git(cwd, &["symbolic-ref", "--short", "HEAD"]).await;
            if !ok || !branch.trim().starts_with("wks/") {
                return Ok(None);
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(Some(Tree {
        cwd: cwd.to_path_buf(),
        git_dir,
        created,
        identity,
    }))
}

/// The artifact is still the plain directory at its own canonical path.
fn unchanged_directory(candidate: &Path) -> Result<Option<(u64, u64)>> {
    let metadata = std::fs::symlink_metadata(candidate)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || std::fs::canonicalize(candidate)? != candidate
    {
        return Ok(None);
    }
    Ok(Some(identity(&metadata)?))
}

async fn reclaim(
    tree: &Tree,
    candidate: &Path,
    references: &[Reference],
    options: &Options<'_>,
    item: &mut Artifact,
) -> Result<()> {
    if !contains(&tree.cwd, candidate) || candidate == tree.cwd {
        bail!("Artifact path left its worktree");
    }
    if references.iter().any(|reference| {
        !contains(candidate, &reference.source) && overlaps(candidate, &reference.target)
    }) {
        bail!("Referenced by a managed worktree symlink");
    }
    let relative = candidate
        .strip_prefix(&tree.cwd)?
        .components()
        .map(|part| {
            part.as_os_str()
                .to_str()
                .ok_or_else(|| anyhow!("Artifact path is not Unicode"))
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    // Literal pathspecs: a bracket or star in a parent directory's name must
    // name that directory, not match (or fail to match) as a pattern.
    let (ok, tracked) = git(
        &tree.cwd,
        &["--literal-pathspecs", "ls-files", "-z", "--", &relative],
    )
    .await;
    if !ok || !tracked.is_empty() {
        bail!("Contains tracked files or Git state is unavailable");
    }
    // check-ignore takes path names, not pathspecs, and rejects that option.
    let (ignored, _) = git(&tree.cwd, &["check-ignore", "-q", "--", &relative]).await;
    if !ignored {
        bail!("Not Git-ignored");
    }
    let Some(original) = unchanged_directory(candidate)? else {
        bail!("Artifact path changed or traverses a symlink");
    };
    let (bytes, modified) = measure(candidate, options)?;
    item.bytes = bytes;
    if !aged(modified, options.min_age) {
        bail!("Artifact was recently written; waiting for cleanup cooldown");
    }
    if !options.apply {
        item.action = Action::Planned;
        return Ok(());
    }
    proceed(options)?;
    if unchanged_directory(candidate)? != Some(original) {
        bail!("Artifact identity changed during inspection");
    }
    std::fs::remove_dir_all(candidate)?;
    item.action = Action::Removed;
    Ok(())
}

async fn clean_tree<F, Fut>(
    tree: &Tree,
    options: &Options<'_>,
    load_state: &F,
    report: &mut CleanupReport,
) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<CleanupState>>,
{
    proceed(options)?;
    // Cross-process exclusive lease shared with daemon spawn admission. Never
    // stolen by age: a slow cleanup must not race a newly admitted provider.
    // Dropped (and its still-owned file released) on every exit below.
    let _lease = WorktreeMaintenance::acquire_git_dir(&tree.git_dir).map_err(|error| {
        if error.kind() == ErrorKind::AlreadyExists {
            anyhow!(
                "Worktree maintenance or another launch is in progress; retry after it finishes"
            )
        } else {
            anyhow!("Worktree maintenance lease is unavailable: {error}")
        }
    })?;
    let current = std::fs::symlink_metadata(&tree.cwd)?;
    if linked_worktree(&tree.cwd)? != Some((tree.cwd.clone(), tree.git_dir.clone()))
        || current.file_type().is_symlink()
        || identity(&current)? != tree.identity
    {
        bail!("Worktree identity changed before cleanup");
    }
    if exists_no_follow(&tree.git_dir.join("locked"))? {
        bail!("Git worktree is locked; cleanup skipped");
    }
    let state = load_state().await?;
    if options.apply && !state.maintenance_supported {
        bail!("Daemon lacks worktree-maintenance protection; update it before cleanup");
    }
    let reason = inactivity_reason(&tree.cwd, &state, options.min_age);
    if reason.is_some() || !aged(tree.created, options.min_age) {
        report.skipped.push(Skipped {
            worktree: tree.cwd.display().to_string(),
            reason: reason
                .unwrap_or("New worktree; waiting for cleanup cooldown")
                .into(),
        });
        return Ok(());
    }
    // Re-discover under the source lease: a child worktree could have linked
    // these dependencies since initial enumeration, and worktree creation
    // holds this same lease while creating those links. Unreadable sibling
    // trees refuse cleanup because sharing cannot be ruled out.
    let references = shared_references(&tree.cwd, options).await?;
    for candidate in scan(&tree.cwd, options)? {
        let mut item = Artifact {
            worktree: tree.cwd.display().to_string(),
            path: candidate.display().to_string(),
            bytes: 0,
            action: Action::Skipped,
            reason: None,
        };
        match reclaim(tree, &candidate, &references, options, &mut item).await {
            Ok(()) => {
                if item.action == Action::Removed {
                    report.removed_bytes = report.removed_bytes.saturating_add(item.bytes);
                }
                report.reclaimable_bytes = report.reclaimable_bytes.saturating_add(item.bytes);
            }
            Err(error) => item.reason = Some(format!("{error:#}")),
        }
        report.artifacts.push(item);
    }
    Ok(())
}

/// One sweep of `options.root`, laid out as `<root>/<repository>/<worktree>`.
/// `load_state` is called once per eligible worktree, under its lease.
pub async fn cleanup_worktree_artifacts<F, Fut>(
    options: Options<'_>,
    load_state: F,
) -> Result<CleanupReport>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<CleanupState>>,
{
    let root = canonical(options.root);
    let mut report = CleanupReport {
        root: root.display().to_string(),
        apply: options.apply,
        scanned_worktrees: 0,
        removed_bytes: 0,
        reclaimable_bytes: 0,
        artifacts: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
    };
    let groups = match entries(&root) {
        Ok(groups) => groups,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            return Ok(report);
        }
        Err(error) => return Err(error),
    };
    let mut worktrees = Vec::new();
    for (repository, kind) in groups {
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let group = root.join(repository);
        for (child, kind) in entries(&group)? {
            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }
            let cwd = group.join(child);
            match managed_worktree(&cwd, &root).await {
                Ok(Some(tree)) => worktrees.push(tree),
                Ok(None) => {}
                Err(error) => report.skipped.push(Skipped {
                    worktree: cwd.display().to_string(),
                    reason: format!("{error:#}"),
                }),
            }
        }
    }
    report.scanned_worktrees = worktrees.len();
    for tree in &worktrees {
        if let Err(error) = clean_tree(tree, &options, &load_state, &mut report).await {
            report.skipped.push(Skipped {
                worktree: tree.cwd.display().to_string(),
                reason: format!("{error:#}"),
            });
        }
    }
    Ok(report)
}

#[cfg(all(test, unix))]
#[path = "artifact_cleanup_tests.rs"]
mod tests;
