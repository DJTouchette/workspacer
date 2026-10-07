//! Immutable review evidence tied to one host-owned worktree allocation.
//! Reads select retained IDs only; they never accept a repository or revision.
use super::{atomic_json, files::GIT_NO_EXEC, paths};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
// Retained public evidence intentionally uses a generic reason. Diagnostics
// identify typed failure stages/errno without printing Git output, file content,
// argv, or credentials that an arbitrary error Display could contain.
fn capture_diagnostic(error: &anyhow::Error) -> String {
    let mut tags = Vec::new();
    for cause in error.chain() {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            tags.push(format!(
                "io(kind={:?},errno={:?})",
                io.kind(),
                io.raw_os_error()
            ));
        } else if cause
            .downcast_ref::<super::owned_process::OutputLimit>()
            .is_some()
        {
            tags.push("output-limit".into());
        } else {
            match cause.to_string().as_str() {
                "command cleanup outcome is unknown" => tags.push("cleanup-unconfirmed".into()),
                "command anchor was reaped; refusing a numeric group signal" => {
                    tags.push("anchor-check".into())
                }
                "command cleanup did not confirm child exit" => {
                    tags.push("reap-unconfirmed".into())
                }
                "command does not own a separate process group" => {
                    tags.push("group-ownership".into())
                }
                "review capture deadline exceeded" => tags.push("capture-timeout".into()),
                _ => (),
            }
        }
    }
    if tags.is_empty() {
        "unclassified-capture-failure".into()
    } else {
        tags.join(" -> ")
    }
}
const TOTAL: usize = 24 * 1024 * 1024;
const DIFF: usize = 1024 * 1024;
#[derive(Clone, Serialize, Deserialize, Default)]
struct State {
    allocations: Vec<Value>,
    records: Vec<Value>,
    #[serde(default)]
    readers: BTreeMap<String, Vec<String>>,
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn sha(value: &str) -> bool {
    (40..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Debug)]
struct Oversized;
impl std::fmt::Display for Oversized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("review capture size limit exceeded")
    }
}
impl std::error::Error for Oversized {}
async fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(cwd)
        .args(["--no-replace-objects", "--literal-pathspecs"]);
    for key in GIT_NO_EXEC {
        command.args(["-c", key]);
    }
    command
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0");
    let output = super::owned_process::capture(&mut command, DIFF, Duration::from_secs(15)).await?;
    anyhow::ensure!(output.status.success(), "review Git command failed");
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
type CaptureResult = std::result::Result<Option<String>, String>;
type CaptureCell = tokio::sync::OnceCell<CaptureResult>;
fn spelling(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.as_os_str().to_string_lossy();
        return if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{unc}"))
        } else {
            PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
        };
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}
fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (spelling(a), spelling(b));
    paths::contained(&a, &b) && paths::contained(&b, &a)
}
fn folded_contains(target: &Path, root: &Path) -> bool {
    let target = spelling(target);
    let root = spelling(root);
    paths::contained(
        &PathBuf::from(target.to_string_lossy().to_ascii_lowercase()),
        &PathBuf::from(root.to_string_lossy().to_ascii_lowercase()),
    )
}
pub struct ReviewStore {
    path: PathBuf,
    home: PathBuf,
    config: PathBuf,
    lock: Mutex<()>,
    captures: Mutex<BTreeMap<String, Weak<CaptureCell>>>,
}
impl ReviewStore {
    pub fn new(config: PathBuf, home: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path: config.join("fleet-review.json"),
            config,
            home,
            lock: Mutex::new(()),
            captures: Mutex::new(BTreeMap::new()),
        })
    }
    fn load(&self) -> State {
        self.strict_load().unwrap_or_default()
    }
    fn strict_load(&self) -> Result<State> {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(State::default());
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.len() > TOTAL as u64 {
            bail!("Review history is oversized; ownership was not changed");
        }
        Ok(serde_json::from_slice(&std::fs::read(&self.path)?)?)
    }
    fn save(&self, mut state: State) -> Result<()> {
        if state.allocations.len() > 256 {
            state.allocations.drain(..state.allocations.len() - 256);
        }
        if state.records.len() > 64 {
            state.records.drain(..state.records.len() - 64);
        }
        while !state.records.is_empty() && serde_json::to_vec(&state)?.len() > TOTAL {
            state.records.remove(0);
        }
        state
            .readers
            .retain(|id, _| state.records.iter().any(|record| record["id"] == *id));
        let encoded = serde_json::to_value(state)?;
        anyhow::ensure!(
            serde_json::to_vec(&encoded)?.len() <= TOTAL,
            "Review allocation metadata exceeds the retention budget"
        );
        atomic_json(&self.path, &encoded, true)
    }
    pub fn register(&self, owner: &str, worker: &str, mut allocation: Value) -> Result<()> {
        anyhow::ensure!(
            allocation.is_object() && !owner.is_empty() && !worker.is_empty(),
            "invalid review allocation"
        );
        anyhow::ensure!(
            owner.encode_utf16().count() < 200 && worker.encode_utf16().count() < 200,
            "invalid review identity"
        );
        for key in ["projectRoot", "allocatedCwd", "commonDir", "branch"] {
            anyhow::ensure!(
                !text(&allocation, key).is_empty(),
                "invalid review allocation"
            );
        }
        anyhow::ensure!(sha(text(&allocation, "baseCommit")), "invalid review base");
        let _lock = self.lock.lock().unwrap();
        let mut state = self.load();
        state
            .allocations
            .retain(|row| row["workerSessionId"] != worker);
        allocation["ownerSessionId"] = owner.into();
        allocation["workerSessionId"] = worker.into();
        allocation["generation"] = uuid::Uuid::new_v4().to_string().into();
        state.allocations.push(allocation);
        self.save(state)
    }
    pub fn adopt_owner(&self, old: &str, new: &str) -> Result<()> {
        let _lock = self.lock.lock().unwrap();
        let mut state = self.strict_load()?;
        let mut changed = false;
        for row in &mut state.allocations {
            if row["ownerSessionId"] == old {
                row["ownerSessionId"] = new.into();
                changed = true;
            }
        }
        for record in &state.records {
            let id = text(record, "id");
            let readers = state.readers.entry(id.into()).or_default();
            if (record["ownerSessionId"] == old || readers.iter().any(|id| id == old))
                && !readers.iter().any(|id| id == new)
            {
                readers.push(new.into());
                changed = true;
            }
        }
        if changed {
            self.save(state)?;
        }
        Ok(())
    }
    fn secret(&self, target: &Path) -> bool {
        selected_secret(target, &self.config, &self.home)
    }
    fn restricted(&self, project: &Path, file: &str) -> bool {
        let credential=regex::Regex::new(r"(?i)^(?:\.env(?:\..*)?|\.ssh|\.aws|credentials(?:\.json)?|id_(?:rsa|ed25519)|.*\.(?:pem|p12|pfx|key))$").unwrap();
        file.is_empty()
            || Path::new(file).is_absolute()
            || file.contains('\u{fffd}')
            || file
                .split('/')
                .any(|part| part == ".." || part == ".git" || credential.is_match(part))
            || self.secret(&project.join(file))
    }
    async fn fill(&self, allocation: &Value, evidence: &mut Value) -> Result<()> {
        let cwd = Path::new(text(allocation, "allocatedCwd"));
        let base = text(allocation, "baseCommit");
        anyhow::ensure!(
            sha(base) && same_path(&std::fs::canonicalize(cwd)?, cwd),
            "worktree identity changed"
        );
        let common = std::fs::canonicalize(
            git(
                cwd,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )
            .await?
            .trim(),
        )?;
        let top = std::fs::canonicalize(git(cwd, &["rev-parse", "--show-toplevel"]).await?.trim())?;
        let branch = git(cwd, &["symbolic-ref", "--short", "HEAD"]).await?;
        anyhow::ensure!(
            same_path(&common, Path::new(text(allocation, "commonDir")))
                && same_path(&top, cwd)
                && branch.trim() == text(allocation, "branch"),
            "worktree identity changed"
        );
        let head = git(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])
            .await?
            .trim()
            .to_owned();
        anyhow::ensure!(sha(&head), "invalid head");
        evidence["headCommit"] = head.clone().into();
        git(cwd, &["merge-base", "--is-ancestor", base, &head]).await?;
        let status = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
        if !git(cwd, &status).await?.is_empty() {
            evidence["availability"] = "dirty".into();
            evidence["reason"]="Uncommitted work: review is available only in the retained worktree. No partial committed diff was captured.".into();
            return Ok(());
        }
        let range = format!("{base}..{head}");
        let names = git(
            cwd,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--name-status",
                "-z",
                "-M",
                &range,
                "--",
            ],
        )
        .await?;
        // Validate the whole name list before spawning any per-file diff: one
        // Git process per file is slow on Windows, so learning a range is
        // oversized only after 200 spawns can run past the capture deadline.
        let names: Vec<_> = names.split('\0').collect();
        let mut at = 0;
        let mut entries = Vec::new();
        while at < names.len() && !names[at].is_empty() {
            let status = names[at];
            at += 1;
            let first = *names
                .get(at)
                .ok_or_else(|| anyhow!("incomplete name status"))?;
            at += 1;
            let old = matches!(status.as_bytes().first(), Some(b'R' | b'C')).then_some(first);
            let file = if old.is_some() {
                let file = *names.get(at).ok_or_else(|| anyhow!("incomplete rename"))?;
                at += 1;
                file
            } else {
                first
            };
            if self.restricted(Path::new(text(allocation, "projectRoot")), file)
                || old.is_some_and(|old| {
                    self.restricted(Path::new(text(allocation, "projectRoot")), old)
                })
            {
                bail!("restricted path");
            }
            if entries.len() >= 200 {
                return Err(Oversized.into());
            }
            entries.push((status, old, file));
        }
        let mut bytes = 0;
        let mut files = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        for (status, old, file) in entries {
            if Instant::now() > deadline {
                bail!("review capture deadline exceeded");
            }
            let mut args = vec![
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                &range,
                "--",
            ];
            if let Some(old) = old {
                args.push(old);
            }
            args.push(file);
            let diff = git(cwd, &args).await?;
            bytes += diff.len();
            if bytes > DIFF {
                return Err(Oversized.into());
            }
            let mut row = json!({"path":file,"status":status,"diff":diff});
            if let Some(old) = old {
                row["oldPath"] = old.into();
            }
            files.push(row);
        }
        anyhow::ensure!(
            git(cwd, &status).await?.is_empty()
                && git(cwd, &["rev-parse", "HEAD"]).await?.trim() == head,
            "worktree changed during capture"
        );
        evidence["availability"] = "captured".into();
        evidence["files"] = json!(files);
        Ok(())
    }
    pub async fn capture(
        &self,
        owner: &str,
        worker: &str,
        lifecycle: &str,
    ) -> Result<Option<String>> {
        let allocation = {
            let _lock = self.lock.lock().unwrap();
            self.load()
                .allocations
                .into_iter()
                .find(|row| row["ownerSessionId"] == owner && row["workerSessionId"] == worker)
        };
        let Some(allocation) = allocation else {
            return Ok(None);
        };
        let generation = text(&allocation, "generation");
        let key = format!("{owner}:{worker}:{generation}");
        let cell = {
            let mut captures = self.captures.lock().unwrap();
            captures.retain(|_, value| value.strong_count() > 0);
            match captures.get(&key).and_then(Weak::upgrade) {
                Some(value) => value,
                None => {
                    let value = Arc::new(CaptureCell::new());
                    captures.insert(key, Arc::downgrade(&value));
                    value
                }
            }
        };
        cell.get_or_init(|| async {
            self.capture_now(owner, worker, lifecycle, &allocation)
                .await
                .map_err(|error| error.to_string())
        })
        .await
        .clone()
        .map_err(|error| anyhow!(error))
    }
    async fn capture_now(
        &self,
        owner: &str,
        worker: &str,
        lifecycle: &str,
        allocation: &Value,
    ) -> Result<Option<String>> {
        let generation = text(allocation, "generation");
        let mut evidence = json!({"id":uuid::Uuid::new_v4().to_string(),"allocationId":generation,"ownerSessionId":owner,"workerSessionId":worker,"capturedAt":chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true),"lifecycle":lifecycle,"availability":"unavailable","files":[]});
        for field in ["projectRoot", "allocatedCwd", "branch", "baseCommit"] {
            evidence[field] = allocation[field].clone();
        }
        let capture = tokio::time::timeout(
            Duration::from_secs(30),
            self.fill(allocation, &mut evidence),
        )
        .await
        .map_err(|_| anyhow!("review capture deadline exceeded"))
        .and_then(|result| result);
        if let Err(error) = capture {
            let big = error.downcast_ref::<Oversized>().is_some()
                || error
                    .downcast_ref::<super::owned_process::OutputLimit>()
                    .is_some();
            if !big {
                eprintln!(
                    "review capture {} unavailable: {}",
                    text(&evidence, "id"),
                    capture_diagnostic(&error)
                );
            }
            evidence["files"] = json!([]);
            evidence["availability"] = if big { "oversized" } else { "unavailable" }.into();
            evidence["reason"]=if big{"Review exceeds the 200-file / 1 MiB capture limit; no partial diff retained."}else{"Worktree or commit range unavailable, changed, or contains restricted paths. No fallback repository is used."}.into();
        }
        let _lock = self.lock.lock().unwrap();
        let mut state = self.load();
        let Some(current) = state
            .allocations
            .iter()
            .find(|row| row["workerSessionId"] == worker && row["generation"] == generation)
        else {
            return Ok(None);
        };
        if evidence["availability"] == "unavailable"
            && !Path::new(text(&allocation, "allocatedCwd")).exists()
        {
            if let Some(previous) = state.records.iter().rev().find(|row| {
                row["workerSessionId"] == worker
                    && row["lifecycle"] == "before-worktree-removal"
                    && row["allocationId"] == generation
                    && (row["ownerSessionId"] == owner
                        || state
                            .readers
                            .get(text(row, "id"))
                            .is_some_and(|readers| readers.iter().any(|reader| reader == owner)))
            }) {
                return Ok(Some(text(previous, "id").into()));
            }
        }
        let id = text(&evidence, "id").to_owned();
        if current["ownerSessionId"] != owner {
            state
                .readers
                .insert(id.clone(), vec![text(current, "ownerSessionId").into()]);
        }
        state.records.push(evidence);
        self.save(state)?;
        Ok(Some(id))
    }
    pub async fn before_remove(&self, cwd: &Path) -> Result<()> {
        let cwd = std::fs::canonicalize(cwd)?;
        let entries = {
            let _lock = self.lock.lock().unwrap();
            self.load().allocations
        };
        for row in entries {
            if same_path(Path::new(text(&row, "allocatedCwd")), &cwd) {
                self.capture(
                    text(&row, "ownerSessionId"),
                    text(&row, "workerSessionId"),
                    "before-worktree-removal",
                )
                .await?;
            }
        }
        Ok(())
    }
    fn valid(request: &Value) -> bool {
        let Some(fields) = request.as_object() else {
            return false;
        };
        if fields.keys().any(|key| {
            !matches!(
                key.as_str(),
                "ownerSessionId" | "workerSessionId" | "evidenceId" | "file"
            )
        }) {
            return false;
        }
        if ["ownerSessionId", "workerSessionId", "evidenceId"]
            .iter()
            .any(|key| {
                request[*key]
                    .as_str()
                    .is_none_or(|value| value.is_empty() || value.encode_utf16().count() >= 200)
            })
        {
            return false;
        }
        request.get("file").is_none_or(|file| {
            file.as_str().is_some_and(|file| {
                file.encode_utf16().count() <= 4096
                    && !file.contains('\0')
                    && !file.split(['/', '\\']).any(|part| part == "..")
                    && !Path::new(file).is_absolute()
            })
        })
    }
    fn selected<'a>(state: &'a State, request: &Value) -> Option<&'a Value> {
        state.records.iter().find(|row| {
            row["id"] == request["evidenceId"]
                && row["workerSessionId"] == request["workerSessionId"]
                && (row["ownerSessionId"] == request["ownerSessionId"]
                    || state.readers.get(text(row, "id")).is_some_and(|readers| {
                        readers
                            .iter()
                            .any(|owner| owner == text(request, "ownerSessionId"))
                    }))
        })
    }
    pub fn read(&self, request: &Value) -> Value {
        if !Self::valid(request) {
            return json!({"ok":false,"error":"Invalid review request"});
        }
        let _lock = self.lock.lock().unwrap();
        let state = self.load();
        let Some(record) = Self::selected(&state, request) else {
            return json!({"ok":false,"error":"Review data unavailable, revoked, or outside the owning manager result"});
        };
        let mut record = record.clone();
        let file = request.get("file");
        let files = record["files"].as_array().cloned().unwrap_or_default();
        if file.is_some_and(|file| !files.iter().any(|row| &row["path"] == file)) {
            return json!({"ok":false,"error":"File is not in this captured result"});
        }
        record["files"] = json!(
            files
                .into_iter()
                .filter(|row| file.is_none_or(|file| &row["path"] == file))
                .map(|mut row| {
                    if file.is_none() {
                        if let Some(row) = row.as_object_mut() {
                            row.remove("diff");
                        }
                    }
                    row
                })
                .collect::<Vec<_>>()
        );
        json!({"ok":true,"evidence":record})
    }
    pub fn forget(&self, request: &Value) -> Result<Value> {
        if !Self::valid(request) {
            return Ok(json!({"ok":false}));
        }
        let _lock = self.lock.lock().unwrap();
        let mut state = self.load();
        let Some(selected) = Self::selected(&state, request) else {
            return Ok(json!({"ok":false}));
        };
        if request.get("file").is_some_and(|file| {
            !selected["files"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|row| &row["path"] == file)
        }) {
            return Ok(json!({"ok":false}));
        }
        state
            .records
            .retain(|row| row["id"] != request["evidenceId"]);
        state.allocations.retain(|row| {
            row["ownerSessionId"] != request["ownerSessionId"]
                || row["workerSessionId"] != request["workerSessionId"]
        });
        self.save(state)?;
        Ok(json!({"ok":true}))
    }
}

/// One store instance is shared by launch, cleanup, wakes and ownership transfer.
pub(crate) fn install(
    mut options: crate::Options,
    config: PathBuf,
    home: PathBuf,
) -> crate::Options {
    let store = ReviewStore::new(config, home);
    options.review_store = Some(store.clone());
    for method in ["desktop.fleetReviewRead", "desktop.fleetReviewForget"] {
        let store = store.clone();
        options = options.handler(method, move |caller, params| {
            let store = store.clone();
            async move {
                if !caller.authenticated_host
                    || !caller.trusted
                    || caller.federated
                    || caller.scope != "operator"
                {
                    bail!("Review history requires the server owner");
                }
                let request = params.get("request").cloned().unwrap_or(Value::Null);
                tokio::task::spawn_blocking(move || {
                    if method == "desktop.fleetReviewRead" {
                        Ok(store.read(&request))
                    } else {
                        store.forget(&request)
                    }
                })
                .await?
            }
        });
    }
    options
}

/// Legacy secret-path predicate retained by review evidence and HTML-card
/// selected-object reads. Ambient authenticated fs.* intentionally does not use it.
pub(crate) fn selected_secret(target: &Path, config_dir: &Path, home: &Path) -> bool {
    let Ok(target) = paths::canonicalize(target) else {
        return true;
    };
    let parts: Vec<_> = target
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    let name = parts.last().map(String::as_str).unwrap_or("");
    if matches!(
        name,
        ".bus-token"
            | ".settings.json"
            | ".mcp.json"
            | ".claude.json"
            | "opencode.json"
            | "opencode.jsonc"
            | ".gitconfig"
    ) || parts
        .iter()
        .any(|part| matches!(part.as_str(), ".git" | ".opencode" | ".codex" | ".copilot"))
        || parts.windows(2).any(|pair| {
            (pair[0] == ".claude"
                && matches!(
                    pair[1].as_str(),
                    "settings.json" | "settings.local.json" | "hooks"
                ))
                || (pair[0] == ".github" && pair[1] == "mcp.json")
        })
    {
        return true;
    }
    let Ok(config) = paths::canonicalize(config_dir) else {
        return true;
    };
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    if paths::canonicalize(&xdg.join("git")).is_ok_and(|root| folded_contains(&target, &root)) {
        return true;
    }
    let mut forbidden = vec![PathBuf::from(format!("{}-hub", config_dir.display()))];
    #[cfg(target_os = "macos")]
    forbidden.push(home.join("Library/Application Support/workspacer-hub"));
    if let Ok(global) = paths::canonicalize(&home.join(".gitconfig")) {
        if global == target {
            return true;
        }
    }
    if forbidden
        .drain(..)
        .any(|root| !paths::canonicalize(&root).is_ok_and(|root| !folded_contains(&target, &root)))
    {
        return true;
    }
    if !folded_contains(&target, &config) {
        return false;
    }
    !["library", "sessions", "layouts"].iter().any(|name| {
        paths::canonicalize(&config.join(name)).is_ok_and(|root| {
            root != config && paths::contained(&root, &config) && paths::contained(&target, &root)
        })
    })
}

#[cfg(test)]
mod diagnostic_tests {
    #[test]
    fn capture_diagnostics_expose_errno_but_never_error_output_or_content() {
        let error = anyhow::Error::new(std::io::Error::from_raw_os_error(1))
            .context("credential=PRIVATE diff=PRIVATE")
            .context("command cleanup outcome is unknown");
        let message = super::capture_diagnostic(&error);
        assert!(message.contains("cleanup-unconfirmed"));
        assert!(message.contains("errno=Some(1)"));
        assert!(!message.contains("PRIVATE"));
        assert_eq!(
            super::capture_diagnostic(&anyhow::anyhow!("raw private stderr")),
            "unclassified-capture-failure"
        );
        assert_eq!(
            super::capture_diagnostic(&anyhow::anyhow!("review capture deadline exceeded")),
            "capture-timeout"
        );
        assert_eq!(
            super::capture_diagnostic(&anyhow::Error::new(
                super::super::owned_process::OutputLimit(100)
            )),
            "output-limit"
        );
    }
}
