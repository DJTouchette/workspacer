//! Project scratch: personal working files kept outside every checkout.
//!
//! Design: `docs/features/scratch.md`. The project → directory derivation is
//! `claudemon::scratch` (shared with the spawn-time `WORKSPACER_SCRATCH` env),
//! so an agent's env var and these methods always name the same directory:
//!
//! ```text
//! <home>/.workspacer/scratch/<key>/
//!   project.json   written by claudemon::scratch::ensure
//!   index.json     this service's metadata, keyed by path relative to files/
//!   files/         the scratch files themselves
//! ```
//!
//! Path contract (registration checklist, "selected-object path"): `cwd` is an
//! ambient absolute path canonicalized once and used only to pick the project;
//! `path` is relative, every component is a plain name, and the resolved target
//! must be exactly `files/<path>` after canonicalization — no symlink anywhere
//! below `files/` is followed for a read or a write.
//!
//! There is deliberately no method that runs anything. Scratch can hold prod
//! SQL; the hub records *that* it ran (`status`/`note`), never runs it.
use super::{config::ConfigLock, paths};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::{
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
};

/// Same cap as library items and `files.read`.
const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_LIST: usize = 2000;
const MAX_DEPTH: usize = 8;
const MAX_PATH: usize = 512;
const MAX_PURPOSE: usize = 500;
const MAX_NOTE: usize = 4000;
const STATUSES: &[&str] = &["draft", "pending", "done", "archived"];
/// `scratch.pending` reads at most this many project directories' indexes.
const MAX_PROJECTS: usize = 1000;
/// An index larger than this is skipped by `scratch.pending`, not parsed.
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;

pub struct Scratch {
    home: PathBuf,
}

/// What a mutation publishes on `scratch.changed`: the project's key and its
/// pending count after the write, so a badge updates without a re-read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub key: String,
    pub pending: usize,
}

struct Project {
    root: PathBuf,
    key: String,
    dir: PathBuf,
}
impl Project {
    fn files(&self) -> PathBuf {
        self.dir.join("files")
    }
    fn index(&self) -> PathBuf {
        self.dir.join("index.json")
    }
    fn describe(&self) -> Value {
        json!({
            "root": self.root,
            "key": self.key,
            "dir": self.dir,
            "files": self.files(),
            "exists": self.files().is_dir(),
        })
    }
}

impl Scratch {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    fn project(&self, params: &Value) -> Result<Project> {
        let cwd = string(params, "cwd")?
            .filter(|cwd| !cwd.is_empty())
            .context("cwd is required: the absolute working directory of the project")?;
        let cwd = paths::canonicalize(Path::new(cwd))?;
        let root = claudemon::scratch::project_root(&cwd)
            .with_context(|| format!("cannot resolve a project for {}", cwd.display()))?;
        Ok(Project {
            key: claudemon::scratch::key(&root),
            dir: claudemon::scratch::project_dir(&self.home, &root),
            root,
        })
    }

    pub fn list(&self, params: &Value) -> Result<Value> {
        keys(params, &["cwd"])?;
        let project = self.project(params)?;
        let (index, index_error) = match read_index(&project.index()) {
            Ok(index) => (index, None),
            // A damaged index never hides the files themselves.
            Err(error) => (Map::new(), Some(error.to_string())),
        };
        let mut items = Vec::new();
        let mut truncated = false;
        let files = project.files();
        if files.is_dir() {
            walk(&files, &files, 0, &mut items, &mut truncated)?;
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let items: Vec<Value> = items
            .into_iter()
            .map(|(path, size, modified)| item(&path, index.get(&path), Some((size, modified))))
            .collect();
        let mut result = json!({
            "project": project.describe(),
            "items": items,
            "truncated": truncated,
        });
        if let Some(error) = index_error {
            result["indexError"] = json!(error);
        }
        Ok(result)
    }

    pub fn read(&self, caller_federated: bool, params: &Value) -> Result<Value> {
        keys(params, &["cwd", "path"])?;
        let project = self.project(params)?;
        let rel = relative(params)?;
        let target = existing_file(&project, &rel)?;
        let index = read_index(&project.index())?;
        let key = slash(&rel);
        let meta = index.get(&key);
        let sensitive = meta.is_some_and(|m| m["sensitive"] == true);
        let mut result = item(&key, meta, stat(&target));
        if sensitive && caller_federated {
            result["content"] = Value::Null;
            result["withheld"] =
                json!("sensitive scratch content is not served to federated callers");
            return Ok(result);
        }
        let bytes = super::files::bounded_bytes(&target, MAX_BYTES)?;
        if bytes.contains(&0) {
            bail!("{key} is binary; scratch.read returns text only");
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            anyhow::anyhow!("{key} is not UTF-8 text; scratch.read returns text only")
        })?;
        result["content"] = json!(text);
        Ok(result)
    }

    /// Every project scratch with at least one `status:"pending"` file, read
    /// from each `index.json` only (no file walks). Damaged or missing indexes
    /// are skipped; symlinked project directories are never followed.
    pub fn pending(&self, params: &Value) -> Result<Value> {
        keys(params, &[])?;
        let base = claudemon::scratch::base(&self.home);
        let entries = match std::fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"projects": [], "truncated": false}));
            }
            Err(error) => return Err(error).with_context(|| format!("reading {}", base.display())),
        };
        let mut projects = Vec::new();
        let mut truncated = false;
        for (seen, entry) in entries.flatten().enumerate() {
            if seen >= MAX_PROJECTS {
                truncated = true;
                break;
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let Ok(key) = entry.file_name().into_string() else {
                continue;
            };
            let dir = entry.path();
            let index = dir.join("index.json");
            if !std::fs::symlink_metadata(&index)
                .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_INDEX_BYTES)
            {
                continue;
            }
            let Ok(items) = read_index(&index) else {
                continue;
            };
            let pending = pending_count(&items);
            if pending == 0 {
                continue;
            }
            projects.push(json!({"key": key, "root": project_root(&dir), "pending": pending}));
        }
        projects.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        Ok(json!({"projects": projects, "truncated": truncated}))
    }

    pub fn save(&self, caller_federated: bool, params: &Value) -> Result<(Value, Changed)> {
        keys(
            params,
            &[
                "cwd",
                "path",
                "content",
                "purpose",
                "sessionId",
                "keep",
                "sensitive",
                "status",
                "note",
                "overwrite",
            ],
        )?;
        let project = self.project(params)?;
        let rel = relative(params)?;
        let content = string(params, "content")?.context("content is required")?;
        if content.len() > MAX_BYTES {
            bail!("content exceeds {MAX_BYTES} bytes");
        }
        let purpose = text(params, "purpose", MAX_PURPOSE)?
            .filter(|p| !p.is_empty())
            .context("purpose is required: one line saying what this file is for")?;
        let session = session_id(params)?;
        let keep = boolean(params, "keep")?;
        let asked_sensitive = boolean(params, "sensitive")?;
        let status = status(params)?;
        let note = text(params, "note", MAX_NOTE)?;
        let overwrite = boolean(params, "overwrite")?.unwrap_or(false);

        claudemon::scratch::ensure(&self.home, &project.root)
            .with_context(|| format!("preparing scratch for {}", project.root.display()))?;
        let _lock = ConfigLock::take(&project.index())?;
        let mut index = read_index(&project.index())?;
        let key = slash(&rel);
        let target = writable_target(&project, &rel)?;
        if target.exists() && !overwrite {
            bail!("{key} already exists in scratch; pass overwrite:true to replace it");
        }
        let previous = index.get(&key).cloned().unwrap_or(Value::Null);
        let detected = credential_kinds(content);
        let was_sensitive = previous["sensitive"] == true;
        let sensitive = !detected.is_empty() || asked_sensitive.unwrap_or(was_sensitive);
        if caller_federated && was_sensitive && !sensitive {
            bail!("a federated caller cannot clear the sensitive flag");
        }
        write_file(&target, content.as_bytes(), sensitive)?;

        let now = crate::protocol::now();
        let mut meta = previous.as_object().cloned().unwrap_or_default();
        meta.insert("purpose".into(), json!(purpose));
        if let Some(session) = session {
            meta.insert("sessionId".into(), json!(session));
        }
        meta.insert(
            "keep".into(),
            json!(keep.unwrap_or(previous["keep"] == true)),
        );
        meta.insert("sensitive".into(), json!(sensitive));
        set_status(
            &mut meta,
            status
                .or_else(|| previous["status"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "draft".into()),
            &now,
        );
        if let Some(note) = note {
            meta.insert("note".into(), json!(note));
        }
        meta.entry("createdAt").or_insert_with(|| json!(now));
        meta.insert("updatedAt".into(), json!(now));
        index.insert(key.clone(), Value::Object(meta));
        write_index(&project.index(), &index)?;

        let mut warnings = Vec::new();
        if !detected.is_empty() {
            warnings.push(format!(
                "content looks like it contains a credential ({}); {key} was marked sensitive. Prefer reading secrets from the environment.",
                detected.join(", ")
            ));
        }
        let mut result = item(&key, index.get(&key), stat(&target));
        result["project"] = project.describe();
        result["warnings"] = json!(warnings);
        let changed = Changed {
            pending: pending_count(&index),
            key: project.key,
        };
        Ok((result, changed))
    }

    pub fn mark(&self, caller_federated: bool, params: &Value) -> Result<(Value, Changed)> {
        keys(
            params,
            &[
                "cwd",
                "path",
                "purpose",
                "keep",
                "sensitive",
                "status",
                "note",
            ],
        )?;
        let project = self.project(params)?;
        let rel = relative(params)?;
        let purpose = text(params, "purpose", MAX_PURPOSE)?;
        if purpose.as_deref() == Some("") {
            bail!("purpose cannot be cleared; give a one-line purpose");
        }
        let keep = boolean(params, "keep")?;
        let sensitive = boolean(params, "sensitive")?;
        let status = status(params)?;
        let note = text(params, "note", MAX_NOTE)?;
        if purpose.is_none()
            && keep.is_none()
            && sensitive.is_none()
            && status.is_none()
            && note.is_none()
        {
            bail!("nothing to mark: pass purpose, keep, sensitive, status or note");
        }
        let target = existing_file(&project, &rel)?;
        let _lock = ConfigLock::take(&project.index())?;
        let mut index = read_index(&project.index())?;
        let key = slash(&rel);
        let mut meta = index
            .get(&key)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let was_sensitive = meta.get("sensitive") == Some(&json!(true));
        if caller_federated && was_sensitive && sensitive == Some(false) {
            bail!("a federated caller cannot clear the sensitive flag");
        }
        let now = crate::protocol::now();
        if let Some(purpose) = purpose {
            meta.insert("purpose".into(), json!(purpose));
        }
        if let Some(keep) = keep {
            meta.insert("keep".into(), json!(keep));
        }
        if let Some(sensitive) = sensitive {
            meta.insert("sensitive".into(), json!(sensitive));
            set_mode(&target, sensitive)?;
        }
        if let Some(status) = status {
            set_status(&mut meta, status, &now);
        }
        if let Some(note) = note {
            meta.insert("note".into(), json!(note));
        }
        meta.entry("createdAt").or_insert_with(|| json!(now));
        meta.insert("updatedAt".into(), json!(now));
        index.insert(key.clone(), Value::Object(meta));
        write_index(&project.index(), &index)?;
        let changed = Changed {
            pending: pending_count(&index),
            key: project.key,
        };
        Ok((item(&key, index.get(&key), stat(&target)), changed))
    }
}

// ---- parameters -----------------------------------------------------------

/// Unknown keys are refused, and so is a key that differs from a known one only
/// by case: the checked value must be the used value.
fn keys(params: &Value, allowed: &[&str]) -> Result<()> {
    let Some(object) = params.as_object() else {
        if params.is_null() {
            return Ok(());
        }
        bail!("scratch parameters must be an object");
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            if allowed.iter().any(|a| a.eq_ignore_ascii_case(key)) {
                bail!("parameter {key} differs from a known parameter only by case");
            }
            bail!("unknown parameter {key}");
        }
    }
    Ok(())
}
fn string<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => bail!("{key} must be a string"),
    }
}
fn boolean(params: &Value, key: &str) -> Result<Option<bool>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => bail!("{key} must be a boolean"),
    }
}
/// Bounded human text: trimmed, no control characters except newline/tab.
fn text(params: &Value, key: &str, max: usize) -> Result<Option<String>> {
    let Some(value) = string(params, key)? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.chars().count() > max {
        bail!("{key} exceeds {max} characters");
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        bail!("{key} contains control characters");
    }
    Ok(Some(value.to_owned()))
}
fn session_id(params: &Value) -> Result<Option<String>> {
    let Some(id) = string(params, "sessionId")?.filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    if id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        bail!("sessionId must be a bounded opaque id");
    }
    Ok(Some(id.to_owned()))
}
fn status(params: &Value) -> Result<Option<String>> {
    let Some(status) = string(params, "status")? else {
        return Ok(None);
    };
    if !STATUSES.contains(&status) {
        bail!("status must be one of {}", STATUSES.join(", "));
    }
    Ok(Some(status.to_owned()))
}
fn set_status(meta: &mut Map<String, Value>, status: String, now: &str) {
    if meta.get("status").and_then(Value::as_str) != Some(status.as_str()) {
        meta.insert("statusAt".into(), json!(now));
    }
    meta.insert("status".into(), json!(status));
}

/// `path` as a relative path of plain names only.
fn relative(params: &Value) -> Result<PathBuf> {
    let raw = string(params, "path")?
        .filter(|p| !p.is_empty())
        .context("path is required: a path relative to the project's scratch")?;
    if raw.len() > MAX_PATH || raw.contains(['\0', '\\']) {
        bail!("path must be a relative path using '/' separators");
    }
    let path = Path::new(raw);
    let mut rel = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => rel.push(name),
            Component::CurDir => {}
            _ => bail!("path must stay inside scratch: no absolute paths or '..'"),
        }
    }
    let depth = rel.components().count();
    if depth == 0 || depth > MAX_DEPTH {
        bail!("path must name a file at most {MAX_DEPTH} directories deep");
    }
    Ok(rel)
}
fn slash(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

// ---- filesystem -----------------------------------------------------------

/// The canonical `files/` dir; refuses a symlinked one.
fn files_root(project: &Project) -> Result<PathBuf> {
    let files = project.files();
    if std::fs::symlink_metadata(&files).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("the scratch files directory is a symlink; refusing to follow it");
    }
    paths::canonicalize(&files)
}

/// An existing regular file at exactly `files/<rel>`, reached through no
/// symlink.
fn existing_file(project: &Project, rel: &Path) -> Result<PathBuf> {
    let root = files_root(project)?;
    let expected = root.join(rel);
    let meta = std::fs::symlink_metadata(&expected)
        .with_context(|| format!("{} is not in scratch", slash(rel)))?;
    if !meta.is_file() {
        bail!("{} is not a regular file", slash(rel));
    }
    let resolved = paths::canonicalize(&expected)?;
    if resolved != expected || !paths::contained(&resolved, &root) {
        bail!("{} resolves outside scratch through a link", slash(rel));
    }
    Ok(resolved)
}

/// The target for a write: parent dirs are created inside `files/`, and both
/// the parent and the target must be reached through no symlink.
fn writable_target(project: &Project, rel: &Path) -> Result<PathBuf> {
    let root = files_root(project)?;
    let expected = root.join(rel);
    let parent = expected.parent().context("path names no file")?;
    std::fs::create_dir_all(parent)?;
    if paths::canonicalize(parent)? != parent || !paths::contained(parent, &root) {
        bail!("{} resolves outside scratch through a link", slash(rel));
    }
    match std::fs::symlink_metadata(&expected) {
        Ok(meta) if !meta.is_file() => bail!("{} exists and is not a regular file", slash(rel)),
        _ => Ok(expected),
    }
}

fn write_file(target: &Path, bytes: &[u8], private: bool) -> Result<()> {
    use std::io::Write;
    let parent = target.parent().context("path names no file")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(if private {
                0o600
            } else {
                0o644
            }))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    super::atomic_persist::persist(file, target)?;
    Ok(())
}

fn set_mode(target: &Path, private: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            target,
            std::fs::Permissions::from_mode(if private { 0o600 } else { 0o644 }),
        )?;
    }
    #[cfg(not(unix))]
    let _ = (target, private);
    Ok(())
}

fn stat(path: &Path) -> Option<(u64, Option<String>)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.len(), modified(&meta)))
}
fn modified(meta: &std::fs::Metadata) -> Option<String> {
    let time: time::OffsetDateTime = meta.modified().ok()?.into();
    time.format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// Regular files under `files/`, depth- and count-bounded; symlinks are listed
/// as nothing (neither followed nor reported).
fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(String, u64, Option<String>)>,
    truncated: &mut bool,
) -> Result<()> {
    if depth >= MAX_DEPTH {
        *truncated = true;
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= MAX_LIST {
            *truncated = true;
            return Ok(());
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.is_dir() {
            walk(root, &entry.path(), depth + 1, out, truncated)?;
        } else if meta.is_file() {
            let Ok(rel) = entry.path().strip_prefix(root).map(Path::to_path_buf) else {
                continue;
            };
            out.push((slash(&rel), meta.len(), modified(&meta)));
        }
    }
    Ok(())
}

fn read_index(path: &Path) -> Result<Map<String, Value>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(error.into()),
    };
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("scratch index {} is not valid JSON", path.display()))?;
    match value.get("items") {
        Some(Value::Object(items)) => Ok(items.clone()),
        _ => bail!("scratch index {} has no items object", path.display()),
    }
}
fn pending_count(items: &Map<String, Value>) -> usize {
    items
        .values()
        .filter(|meta| meta["status"] == "pending")
        .count()
}
/// The root recorded in a project's `project.json`, or null when unreadable.
fn project_root(dir: &Path) -> Value {
    let record = dir.join("project.json");
    if !std::fs::symlink_metadata(&record)
        .is_ok_and(|meta| meta.is_file() && meta.len() <= 64 * 1024)
    {
        return Value::Null;
    }
    std::fs::read(&record)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|record| record.get("root").filter(|root| root.is_string()).cloned())
        .unwrap_or(Value::Null)
}
fn write_index(path: &Path, items: &Map<String, Value>) -> Result<()> {
    super::atomic_json(path, &json!({"version": 1, "items": items}), true)
}

/// One list/read/save row: the file's own facts plus its metadata. A file an
/// agent wrote directly (no index entry) is `indexed:false` with no purpose.
fn item(path: &str, meta: Option<&Value>, file: Option<(u64, Option<String>)>) -> Value {
    let meta = meta.cloned().unwrap_or(Value::Null);
    let field = |key: &str| meta.get(key).cloned().unwrap_or(Value::Null);
    let (size, modified) = file.unwrap_or((0, None));
    json!({
        "path": path,
        "size": size,
        "modifiedAt": modified,
        "indexed": meta.is_object(),
        "purpose": field("purpose"),
        "sessionId": field("sessionId"),
        "keep": meta.get("keep") == Some(&json!(true)),
        "sensitive": meta.get("sensitive") == Some(&json!(true)),
        "status": field("status"),
        "statusAt": field("statusAt"),
        "note": field("note"),
        "createdAt": field("createdAt"),
        "updatedAt": field("updatedAt"),
    })
}

/// Names of credential shapes found in `content` — never the matched text.
fn credential_kinds(content: &str) -> Vec<&'static str> {
    static PATTERNS: OnceLock<Vec<(&'static str, regex::Regex)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            ("URL with a password", r"(?i)\b[a-z][a-z0-9+.\-]*://[^\s/:@]+:[^\s/@]+@"),
            (
                "password or key assignment",
                r#"(?i)\b(password|passwd|pwd|secret|api[_\-]?key|access[_\-]?token|auth[_\-]?token)\s*[=:]\s*['"]?[^\s'";,)]{4,}"#,
            ),
            ("private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
            ("AWS access key", r"\b(AKIA|ASIA)[0-9A-Z]{16}\b"),
            ("bearer token", r"(?i)\bbearer\s+[a-z0-9._~+/\-]{20,}"),
        ]
        .into_iter()
        .map(|(name, pattern)| (name, regex::Regex::new(pattern).expect("static pattern")))
        .collect()
    });
    patterns
        .iter()
        .filter(|(_, pattern)| pattern.is_match(content))
        .map(|(name, _)| *name)
        .collect()
}

// ---- bus ------------------------------------------------------------------

pub(crate) fn install(
    mut options: crate::Options,
    home: PathBuf,
    hub: crate::Handle,
) -> crate::Options {
    let service = Arc::new(Scratch::new(home));
    for method in [
        "scratch.list",
        "scratch.read",
        "scratch.save",
        "scratch.mark",
        "scratch.pending",
    ] {
        let service = service.clone();
        let hub = hub.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            let hub = hub.clone();
            async move {
                let federated = caller.federated;
                let (result, changed) = tokio::task::spawn_blocking(move || match method {
                    "scratch.list" => service.list(&params).map(|r| (r, None)),
                    "scratch.read" => service.read(federated, &params).map(|r| (r, None)),
                    "scratch.pending" => service.pending(&params).map(|r| (r, None)),
                    "scratch.save" => service.save(federated, &params).map(|(r, c)| (r, Some(c))),
                    _ => service.mark(federated, &params).map(|(r, c)| (r, Some(c))),
                })
                .await??;
                if let Some(Changed { key, pending }) = changed
                    && let Err(error) = hub
                        .publish_wait(crate::protocol::Event::new(
                            "scratch.changed",
                            "brain",
                            json!({"key": key, "pending": pending}),
                        ))
                        .await
                {
                    eprintln!("scratch mutation committed but change notification failed: {error}");
                }
                Ok(result)
            }
        });
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        project: PathBuf,
        scratch: Scratch,
    }
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("shop");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        Fixture {
            scratch: Scratch::new(home.clone()),
            home,
            project,
            _tmp: tmp,
        }
    }
    impl Fixture {
        fn cwd(&self) -> String {
            self.project.to_string_lossy().into_owned()
        }
        fn files(&self) -> PathBuf {
            claudemon::scratch::project_dir(
                &self.home,
                &std::fs::canonicalize(&self.project).unwrap(),
            )
            .join("files")
        }
        fn save(&self, extra: Value) -> Result<Value> {
            let mut params = json!({"cwd": self.cwd(), "path": "load/harness.ts", "content": "run()", "purpose": "load harness"});
            for (k, v) in extra.as_object().unwrap() {
                params[k] = v.clone();
            }
            self.scratch.save(false, &params).map(|(r, _)| r)
        }
    }

    #[test]
    fn save_then_list_and_read_round_trip_with_metadata() {
        let f = fixture();
        let saved = f
            .save(json!({"sessionId": "abc-123", "status": "pending", "keep": true}))
            .unwrap();
        assert_eq!(saved["path"], "load/harness.ts");
        assert_eq!(saved["status"], "pending");
        assert_eq!(saved["keep"], true);
        assert_eq!(saved["warnings"], json!([]));
        assert!(f.files().join("load/harness.ts").is_file());

        let list = f.scratch.list(&json!({"cwd": f.cwd()})).unwrap();
        assert_eq!(list["project"]["exists"], true);
        assert_eq!(list["items"].as_array().unwrap().len(), 1);
        let row = &list["items"][0];
        assert_eq!(row["purpose"], "load harness");
        assert_eq!(row["sessionId"], "abc-123");
        assert_eq!(row["indexed"], true);

        let read = f
            .scratch
            .read(false, &json!({"cwd": f.cwd(), "path": "load/harness.ts"}))
            .unwrap();
        assert_eq!(read["content"], "run()");
    }

    #[test]
    fn files_written_directly_are_listed_unindexed_and_can_be_marked() {
        let f = fixture();
        claudemon::scratch::ensure(&f.home, &f.project).unwrap();
        std::fs::write(f.files().join("repro.sh"), "echo hi").unwrap();
        let list = f.scratch.list(&json!({"cwd": f.cwd()})).unwrap();
        assert_eq!(list["items"][0]["path"], "repro.sh");
        assert_eq!(list["items"][0]["indexed"], false);
        assert_eq!(list["items"][0]["purpose"], Value::Null);

        let (marked, changed) = f
            .scratch
            .mark(false, &json!({"cwd": f.cwd(), "path": "repro.sh", "purpose": "cart repro", "status": "done", "note": "fixed in #12"}))
            .unwrap();
        assert!(changed.key.starts_with("shop-"));
        assert_eq!(changed.pending, 0);
        assert_eq!(marked["indexed"], true);
        assert_eq!(marked["status"], "done");
        assert!(marked["statusAt"].is_string());
        assert_eq!(
            std::fs::read_to_string(f.files().join("repro.sh")).unwrap(),
            "echo hi"
        );
    }

    #[test]
    fn listing_a_project_without_scratch_is_empty_and_creates_nothing() {
        let f = fixture();
        let list = f.scratch.list(&json!({"cwd": f.cwd()})).unwrap();
        assert_eq!(list["items"], json!([]));
        assert_eq!(list["project"]["exists"], false);
        assert!(!claudemon::scratch::base(&f.home).exists());
    }

    #[test]
    fn overwrite_is_explicit_and_metadata_survives_it() {
        let f = fixture();
        f.save(json!({"keep": true})).unwrap();
        let error = f.save(json!({"content": "v2"})).unwrap_err().to_string();
        assert!(error.contains("overwrite:true"), "{error}");
        let again = f
            .save(json!({"content": "v2", "overwrite": true, "purpose": "load harness v2"}))
            .unwrap();
        assert_eq!(again["keep"], true, "keep is not reset by a rewrite");
        assert_eq!(again["purpose"], "load harness v2");
        assert_eq!(
            std::fs::read_to_string(f.files().join("load/harness.ts")).unwrap(),
            "v2"
        );
    }

    #[test]
    fn purpose_is_required_and_parameters_are_strict() {
        let f = fixture();
        let missing = f.save(json!({"purpose": "  "})).unwrap_err().to_string();
        assert!(missing.contains("purpose is required"), "{missing}");
        let unknown = f.save(json!({"execute": true})).unwrap_err().to_string();
        assert!(unknown.contains("unknown parameter"), "{unknown}");
        let case = f.save(json!({"Path": "x"})).unwrap_err().to_string();
        assert!(case.contains("only by case"), "{case}");
        let status = f
            .save(json!({"status": "ran-on-prod"}))
            .unwrap_err()
            .to_string();
        assert!(status.contains("status must be one of"), "{status}");
        let nothing = f
            .scratch
            .mark(false, &json!({"cwd": f.cwd(), "path": "x"}))
            .unwrap_err()
            .to_string();
        assert!(nothing.contains("nothing to mark"), "{nothing}");
        let relative_cwd = f
            .scratch
            .list(&json!({"cwd": "shop"}))
            .unwrap_err()
            .to_string();
        assert!(relative_cwd.contains("absolute"), "{relative_cwd}");
    }

    #[test]
    fn paths_cannot_escape_scratch() {
        let f = fixture();
        for bad in [
            "../x",
            "/etc/passwd",
            "a/../../x",
            "a\\b",
            "",
            "a/b/c/d/e/f/g/h/i",
        ] {
            let error = f.save(json!({"path": bad})).unwrap_err().to_string();
            assert!(
                error.contains("path"),
                "{bad:?} should be refused as a path, got {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_scratch_are_never_followed() {
        let f = fixture();
        f.save(json!({})).unwrap();
        let outside = f._tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "top secret").unwrap();
        std::os::unix::fs::symlink(&outside, f.files().join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), f.files().join("file-link"))
            .unwrap();

        let read_dir = f
            .scratch
            .read(false, &json!({"cwd": f.cwd(), "path": "linked/secret.txt"}))
            .unwrap_err()
            .to_string();
        assert!(read_dir.contains("link"), "{read_dir}");
        let read_file = f
            .scratch
            .read(false, &json!({"cwd": f.cwd(), "path": "file-link"}))
            .unwrap_err()
            .to_string();
        assert!(read_file.contains("not a regular file"), "{read_file}");
        let write = f
            .save(json!({"path": "linked/new.txt"}))
            .unwrap_err()
            .to_string();
        assert!(write.contains("link"), "{write}");
        assert!(!outside.join("new.txt").exists());

        let list = f.scratch.list(&json!({"cwd": f.cwd()})).unwrap();
        let paths: Vec<_> = list["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["path"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(paths, vec!["load/harness.ts"]);
    }

    #[test]
    fn credentials_force_sensitive_private_files_withheld_from_federated_callers() {
        let f = fixture();
        let saved = f
            .save(json!({
                "path": "prod/backfill.sql",
                "content": "-- psql postgres://admin:hunter22@db.prod:5432/shop\nUPDATE orders SET x = 1;",
                "purpose": "backfill to run on prod",
                "status": "pending",
            }))
            .unwrap();
        assert_eq!(saved["sensitive"], true);
        let warning = saved["warnings"][0].as_str().unwrap();
        assert!(warning.contains("URL with a password"), "{warning}");
        assert!(
            !warning.contains("hunter22"),
            "the warning never echoes the secret"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(f.files().join("prod/backfill.sql"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let params = json!({"cwd": f.cwd(), "path": "prod/backfill.sql"});
        let local = f.scratch.read(false, &params).unwrap();
        assert!(local["content"].as_str().unwrap().contains("UPDATE orders"));
        let remote = f.scratch.read(true, &params).unwrap();
        assert_eq!(remote["content"], Value::Null);
        assert!(remote["withheld"].is_string());
        assert_eq!(
            remote["purpose"], "backfill to run on prod",
            "metadata stays visible"
        );

        let clear = json!({"cwd": f.cwd(), "path": "prod/backfill.sql", "sensitive": false});
        let refused = f.scratch.mark(true, &clear).unwrap_err().to_string();
        assert!(refused.contains("federated"), "{refused}");
        let (cleared, _) = f.scratch.mark(false, &clear).unwrap();
        assert_eq!(cleared["sensitive"], false);
    }

    #[test]
    fn credential_shapes() {
        assert_eq!(credential_kinds("SELECT 1;"), Vec::<&str>::new());
        assert_eq!(
            credential_kinds("see https://example.com/a@b"),
            Vec::<&str>::new()
        );
        assert_eq!(
            credential_kinds("Server=x;Password=s3cr3t!;"),
            vec!["password or key assignment"]
        );
        assert_eq!(
            credential_kinds("export API_KEY='abcd1234'"),
            vec!["password or key assignment"]
        );
        assert_eq!(
            credential_kinds("-----BEGIN OPENSSH PRIVATE KEY-----"),
            vec!["private key"]
        );
        assert_eq!(
            credential_kinds("AKIAABCDEFGHIJKLMNOP"),
            vec!["AWS access key"]
        );
        assert_eq!(
            credential_kinds("Authorization: Bearer abcdefghijklmnopqrstuvwxyz"),
            vec!["bearer token"]
        );
    }

    #[test]
    fn pending_counts_come_from_each_index_and_mutations_report_them() {
        let f = fixture();
        assert_eq!(
            f.scratch.pending(&json!({})).unwrap(),
            json!({"projects": [], "truncated": false}),
            "no scratch directory at all is an empty answer"
        );
        let (_, changed) = f
            .scratch
            .save(false, &json!({"cwd": f.cwd(), "path": "a.sql", "content": "SELECT 1;", "purpose": "a", "status": "pending"}))
            .unwrap();
        assert_eq!(changed.pending, 1);
        let (_, changed) = f
            .scratch
            .save(false, &json!({"cwd": f.cwd(), "path": "b.sql", "content": "SELECT 2;", "purpose": "b", "status": "pending"}))
            .unwrap();
        assert_eq!(changed.pending, 2);
        f.save(json!({"path": "notes.md"})).unwrap();

        // A second project with nothing pending, one with a damaged index, and a
        // stray file at the base are all left out.
        let other = f._tmp.path().join("other");
        std::fs::create_dir_all(other.join(".git")).unwrap();
        f.scratch
            .save(false, &json!({"cwd": other.to_string_lossy(), "path": "x.txt", "content": "x", "purpose": "x"}))
            .unwrap();
        let base = claudemon::scratch::base(&f.home);
        std::fs::create_dir_all(base.join("broken-0")).unwrap();
        std::fs::write(base.join("broken-0/index.json"), "{not json").unwrap();
        std::fs::write(base.join("stray.txt"), "x").unwrap();

        let pending = f.scratch.pending(&json!({})).unwrap();
        let projects = pending["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1, "{pending}");
        let key = f
            .files()
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(projects[0]["key"], key.as_str());
        assert_eq!(projects[0]["pending"], 2);
        assert_eq!(
            projects[0]["root"],
            json!(std::fs::canonicalize(&f.project).unwrap())
        );

        let (_, changed) = f
            .scratch
            .mark(
                false,
                &json!({"cwd": f.cwd(), "path": "a.sql", "status": "done"}),
            )
            .unwrap();
        assert_eq!(
            changed,
            Changed {
                key: key.clone(),
                pending: 1
            }
        );
        assert_eq!(
            f.scratch.pending(&json!({})).unwrap()["projects"][0]["pending"],
            1
        );
        let strict = f
            .scratch
            .pending(&json!({"cwd": f.cwd()}))
            .unwrap_err()
            .to_string();
        assert!(strict.contains("unknown parameter"), "{strict}");
    }

    #[cfg(unix)]
    #[test]
    fn pending_never_follows_a_symlinked_project_directory() {
        let f = fixture();
        f.save(json!({"status": "pending"})).unwrap();
        let base = claudemon::scratch::base(&f.home);
        let real = f.files().parent().unwrap().to_path_buf();
        std::os::unix::fs::symlink(&real, base.join("linked-0")).unwrap();
        let pending = f.scratch.pending(&json!({})).unwrap();
        assert_eq!(
            pending["projects"].as_array().unwrap().len(),
            1,
            "{pending}"
        );
    }

    #[test]
    fn a_damaged_index_never_hides_files_and_is_never_clobbered() {
        let f = fixture();
        f.save(json!({})).unwrap();
        let index = f.files().parent().unwrap().join("index.json");
        std::fs::write(&index, "{not json").unwrap();
        let list = f.scratch.list(&json!({"cwd": f.cwd()})).unwrap();
        assert_eq!(list["items"][0]["path"], "load/harness.ts");
        assert!(list["indexError"].is_string());
        assert!(f.save(json!({"path": "other.txt"})).is_err());
        assert_eq!(std::fs::read_to_string(&index).unwrap(), "{not json");
    }
}
