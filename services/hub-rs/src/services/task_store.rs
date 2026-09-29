//! Durable task/request history shared by workflow admission and manager inboxes.
//! Caller parameters never establish ownership; authorization closures run again
//! after the cross-process lock and reload, before any mutation commits.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
};
mod admission;
mod directory;
mod host;
pub(crate) mod project;
use project::same_cwd;
pub mod references;
mod remote;
pub use admission::validate_admission;
pub type OwnerLookup = std::sync::Arc<dyn Fn(&str) -> Option<Value> + Send + Sync>;
pub(crate) fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub(crate) fn remove(v: &mut Value, key: &str) {
    if let Some(m) = v.as_object_mut() {
        m.remove(key);
    }
}
pub fn revision(task: &Value) -> u64 {
    task["revision"].as_u64().unwrap_or(0)
}
pub fn manager(owner: &Value) -> Result<()> {
    if owner["isWakeTarget"] != true
        || !text(&owner["hub"]).is_empty()
        || owner["status"] == "ended"
        || text(&owner["sessionId"]).is_empty()
    {
        bail!("requires a live local manager");
    }
    Ok(())
}
pub fn terminal(state: &Value) -> bool {
    matches!(text(state), "completed" | "skipped" | "waived")
}
pub fn task_outcome_accepted(task: &Value) -> bool {
    let accepted = &task["acceptedOutcome"];
    if task["cancelled"] == true
        || accepted["evidence"].as_array().is_none_or(Vec::is_empty)
        || !task["workflow"].is_object()
    {
        return false;
    }
    if !source_delivered(task) {
        return false;
    }
    let Some(steps) = task["workflow"]["steps"].as_array() else {
        return false;
    };
    if !steps.iter().any(|s| s["state"] == "completed")
        || steps
            .iter()
            .any(|s| !matches!(text(&s["state"]), "completed" | "skipped"))
    {
        return false;
    }
    steps.iter().filter(|s| s["state"] == "completed").all(|s| {
        accepted["evidence"].as_array().unwrap().iter().any(|e| {
            e["stepId"] == s["id"]
                && e["dispatchId"] == s["dispatchId"]
                && e["outcome"] == s["outcome"]
        }) && task["attempts"].as_array().into_iter().flatten().any(|a| {
            a["dispatchId"] == s["dispatchId"]
                && a["sessionId"] == s["sessionId"]
                && a["resultContract"] == "valid"
        })
    })
}
fn source_delivered(task: &Value) -> bool {
    task["sources"].as_array().is_none_or(|s| {
        s.is_empty()
            || s.iter()
                .any(|s| matches!(text(&s["delivery"]), "accepted" | "unknown"))
    })
}
pub fn dependency_state(task: &Value, tasks: &[Value]) -> &'static str {
    fn ready(task: &Value, tasks: &[Value], visited: &mut BTreeSet<String>) -> bool {
        if !visited.insert(text(&task["taskId"]).into())
            || task["cancelled"] == true
            || !source_delivered(task)
        {
            return false;
        }
        let result = task["dependsOn"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|id| {
                tasks.iter().find(|t| t["taskId"] == *id).is_some_and(|t| {
                    t["ownerSessionId"] == task["ownerSessionId"]
                        && same_cwd(text(&t["projectCwd"]), text(&task["projectCwd"]))
                        && task_outcome_accepted(t)
                        && ready(t, tasks, visited)
                })
            });
        visited.remove(text(&task["taskId"]));
        result
    }
    if task["cancelled"] == true {
        "cancelled"
    } else if ready(task, tasks, &mut BTreeSet::new()) {
        "ready"
    } else {
        "blocked"
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct History {
    pub version: u32,
    pub tasks: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<Value>,
}
impl Default for History {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: vec![],
            requests: vec![],
        }
    }
}
impl History {
    pub fn task(&self, id: &str) -> Result<&Value> {
        self.tasks
            .iter()
            .find(|t| t["taskId"] == id)
            .context("Task is no longer available")
    }
    pub fn task_mut(&mut self, id: &str) -> Result<&mut Value> {
        self.tasks
            .iter_mut()
            .find(|t| t["taskId"] == id)
            .context("Task is no longer available")
    }
    pub fn owned(&self, id: &str, owner: &str, cwd: &str) -> Result<&Value> {
        let t = self.task(id)?;
        if t["ownerSessionId"] != owner || !same_cwd(text(&t["projectCwd"]), cwd) {
            bail!("Task unavailable for this manager/project");
        }
        Ok(t)
    }
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub tasks: usize,
    pub attempts: usize,
    pub bytes: usize,
    pub requests: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            tasks: 200,
            attempts: 1000,
            bytes: 2 * 1024 * 1024,
            requests: 256,
        }
    }
}
pub struct TaskStore {
    path: PathBuf,
    lock: Mutex<()>,
    fresh: Mutex<BTreeMap<String, String>>,
    observations: Mutex<BTreeMap<String, Value>>,
    limits: Limits,
}
impl TaskStore {
    pub fn open(path: PathBuf) -> Result<Self> {
        let store = Self {
            path,
            lock: Mutex::new(()),
            fresh: Mutex::new(BTreeMap::new()),
            observations: Mutex::new(BTreeMap::new()),
            limits: Limits::default(),
        };
        store.read()?;
        Ok(store)
    }
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
    fn read(&self) -> Result<History> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(History::default()),
            Err(e) => return Err(e.into()),
        };
        if bytes.len() > self.limits.bytes {
            bail!("Task history exceeds its size limit");
        }
        let state: History =
            serde_json::from_slice(&bytes).context("Task history format unavailable")?;
        validate(&state)?;
        Ok(state)
    }
    pub fn transaction<T>(&self, change: impl FnOnce(&mut History) -> Result<T>) -> Result<T> {
        let _local = self.lock.lock().unwrap();
        std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))?;
        // Stable inode: unlinking an advisory lock lets two processes acquire
        // different inodes. It intentionally does not reuse legacy O_EXCL locks.
        let lock_base = PathBuf::from(format!("{}.rust", self.path.display()));
        let _file = crate::auth::StoreLock::take(&lock_base)
            .context("Task history write lock unavailable")?;
        let before = self.read()?;
        let mut next = before.clone();
        let result = change(&mut next)?;
        for task in &mut next.tasks {
            let old = before.tasks.iter().find(|t| t["taskId"] == task["taskId"]);
            if old != Some(task) {
                task["revision"] = old
                    .map(revision)
                    .unwrap_or(0)
                    .checked_add(1)
                    .context("task revision exhausted")?
                    .into();
            }
        }
        validate(&next)?;
        self.prune(&mut next)?;
        if serde_json::to_value(&before)? != serde_json::to_value(&next)? {
            super::atomic_json(&self.path, &serde_json::to_value(&next)?, true)?;
        }
        Ok(result)
    }
    fn prune(&self, state: &mut History) -> Result<()> {
        while state.requests.len() > self.limits.requests {
            let at = state
                .requests
                .iter()
                .position(|r| {
                    (r.get("intents").is_some() || r["delivery"] == "rejected")
                        && !state.tasks.iter().any(|t| {
                            t["sources"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|s| s["requestId"] == r["requestId"])
                        })
                })
                .context("Request inbox capacity reached; resolve pending requests")?;
            state.requests.remove(at);
        }
        while state.tasks.len() > self.limits.tasks
            || state
                .tasks
                .iter()
                .map(|t| t["attempts"].as_array().map(Vec::len).unwrap_or(0))
                .sum::<usize>()
                > self.limits.attempts
            || serde_json::to_vec(state)?.len() > self.limits.bytes
        {
            let at = state
                .tasks
                .iter()
                .position(|t| {
                    let live = t["attempts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|a| a["live"] == true);
                    (t["sources"].as_array().is_none_or(Vec::is_empty) || !live)
                        && !state.tasks.iter().any(|other| {
                            other["dependsOn"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|id| *id == t["taskId"])
                        })
                        && !state.requests.iter().any(|r| {
                            r.get("intents").is_none() && r["ownerSessionId"] == t["ownerSessionId"]
                        })
                        && (t.get("workflow").is_none()
                            || t["workflow"]["steps"]
                                .as_array()
                                .is_some_and(|s| s.iter().all(|s| terminal(&s["state"]))))
                        && t.get("dispatchReservation").is_none()
                })
                .context("Workflow history capacity reached; active tasks cannot be evicted")?;
            state.tasks.remove(at);
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Result<History> {
        let _local = self.lock.lock().unwrap();
        self.read()
    }
    pub fn task(&self, id: &str) -> Result<Option<Value>> {
        let state = self.snapshot()?;
        Ok(state
            .tasks
            .iter()
            .find(|t| t["taskId"] == id)
            .map(|t| self.view(t)))
    }
    pub fn list(&self) -> Result<Vec<Value>> {
        Ok(self
            .snapshot()?
            .tasks
            .iter()
            .rev()
            .map(|t| self.view(t))
            .collect())
    }
    pub fn requests(&self, owner: &str) -> Result<Vec<Value>> {
        Ok(self
            .snapshot()?
            .requests
            .into_iter()
            .filter(|r| r["ownerSessionId"] == owner)
            .collect())
    }
    fn view(&self, task: &Value) -> Value {
        let fresh = self.fresh.lock().unwrap();
        let mut task = task.clone();
        for a in task["attempts"].as_array_mut().into_iter().flatten() {
            if fresh.get(text(&a["sessionId"])).map(String::as_str) != a["observedAt"].as_str() {
                a["stale"] = true.into();
                a["live"] = false.into();
            }
        }
        task
    }
    pub fn mutate_owned(
        &self,
        id: &str,
        owner: &str,
        cwd: &str,
        expected: u64,
        authorize: impl FnOnce() -> Result<()>,
        edit: impl FnOnce(&mut Value) -> Result<()>,
    ) -> Result<Value> {
        self.transaction(|state| {
            authorize()?;
            let task = state.owned(id, owner, cwd)?;
            if revision(task) != expected {
                bail!("conflict: task changed; reload before applying mutation");
            }
            edit(state.task_mut(id)?)
        })?;
        let task = self.task(id)?.context("Task was removed")?;
        if task["ownerSessionId"] != owner || !same_cwd(text(&task["projectCwd"]), cwd) {
            bail!("Task ownership changed after mutation");
        }
        Ok(task)
    }
    pub fn start_workflow(
        &self,
        owner: &Value,
        cwd: &str,
        title: &str,
        workflow: Value,
    ) -> Result<Value> {
        manager(owner)?;
        let id = uuid::Uuid::new_v4().to_string();
        self.transaction(|state|{if state.requests.iter().any(|r|r["ownerSessionId"]==owner["sessionId"]&&r.get("intents").is_some()){bail!("Resolve an inbox request to create new work; existing tasks keep their IDs");}
            state.tasks.push(json!({"taskId":id,"ownerSessionId":owner["sessionId"],"ownerLabel":owner.get("label").unwrap_or(&owner["sessionId"]),"projectCwd":cwd,"title":title.chars().take(300).collect::<String>(),"createdAt":now(),"attempts":[],"workflow":workflow}));Ok(())})?;
        self.task(&id)?.context("Task was removed")
    }
    pub fn reserve_workflow_dispatch(
        &self,
        id: &str,
        expected: u64,
        step_id: &str,
    ) -> Result<String> {
        self.transaction(|state| {
            let task = state.task_mut(id)?;
            if revision(task) != expected {
                bail!("conflict: task changed before dispatch");
            }
            if task.get("dispatchReservation").is_some() {
                bail!("Workflow dispatch already reserved");
            }
            let run = task["workflow"]["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| !terminal(&s["state"]));
            if !run.is_some_and(|s| s["id"] == step_id && s["state"] == "planned") {
                bail!("Workflow step is no longer eligible");
            }
            let token = uuid::Uuid::new_v4().to_string();
            task["dispatchReservation"] = json!({"stepId":step_id,"token":token,"createdAt":now()});
            Ok(token)
        })
    }
    pub fn release_workflow_dispatch(&self, id: &str, token: &str) -> Result<()> {
        self.transaction(|state| {
            if let Ok(task) = state.task_mut(id) {
                if task["dispatchReservation"]["token"] == token {
                    remove(task, "dispatchReservation");
                }
            }
            Ok(())
        })
    }
    pub fn adopt(&self, old: &str, new: &str) -> Result<()> {
        if old.is_empty() || new.is_empty() || old == new {
            bail!("invalid task ownership transfer");
        }
        self.transaction(|state| {
            if state
                .tasks
                .iter()
                .any(|t| t["ownerSessionId"] == old && t.get("dispatchReservation").is_some())
            {
                bail!("Manager adoption must wait for dispatch reservation to settle");
            }
            for t in &mut state.tasks {
                if t["ownerSessionId"] == old {
                    t["ownerSessionId"] = new.into();
                    t["ownerLabel"] = new.into();
                }
            }
            for r in &mut state.requests {
                if r["ownerSessionId"] == old {
                    r["ownerSessionId"] = new.into();
                    r["revision"] = (revision(r) + 1).into();
                }
            }
            Ok(())
        })
    }
    pub fn validated(
        &self,
        session: &str,
        contract: &str,
        evidence_id: Option<&str>,
        outcome: Option<Value>,
    ) -> Result<()> {
        if !["absent", "valid", "invalid", "escalated"].contains(&contract) {
            bail!("invalid result contract state");
        }
        self.transaction(|state| {
            for task in &mut state.tasks {
                let waived = task["workflow"]["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["sessionId"] == session && s["state"] == "waived");
                if waived {
                    continue;
                }
                if let Some(a) = task["attempts"]
                    .as_array_mut()
                    .into_iter()
                    .flatten()
                    .find(|a| a["sessionId"] == session)
                {
                    a["resultContract"] = contract.into();
                    if let Some(id) = evidence_id {
                        a["reviewEvidenceId"] = id.into();
                    }
                    for step in task
                        .get_mut("workflow")
                        .and_then(|w| w.get_mut("steps"))
                        .and_then(Value::as_array_mut)
                        .into_iter()
                        .flatten()
                        .filter(|s| s["sessionId"] == session)
                    {
                        step["state"] = match contract {
                            "valid" => "completed",
                            "escalated" => "blocked",
                            _ => "failed",
                        }
                        .into();
                        if contract == "valid" {
                            remove(step, "reason");
                        } else {
                            step["reason"] = format!("Worker result contract: {contract}").into();
                        }
                        if let Some(v) = &outcome {
                            step["outcome"] = v.clone();
                        } else {
                            remove(step, "outcome");
                        }
                    }
                }
            }
            Ok(())
        })
    }
}
fn validate(state: &History) -> Result<()> {
    if state.version != 1 {
        bail!("Task history format unavailable");
    }
    let mut ids = BTreeSet::new();
    for task in &state.tasks {
        if text(&task["taskId"]).is_empty()
            || text(&task["ownerSessionId"]).is_empty()
            || !task["projectCwd"].is_string()
            || !task["attempts"].is_array()
            || !ids.insert(text(&task["taskId"]))
        {
            bail!("Invalid task history record");
        }
        for a in task["attempts"].as_array().unwrap() {
            if text(&a["dispatchId"]).is_empty()
                || text(&a["sessionId"]).is_empty()
                || !a["metrics"].is_object()
            {
                bail!("Invalid task attempt");
            }
        }
    }
    let mut request_ids = BTreeSet::new();
    for r in &state.requests {
        if text(&r["requestId"]).is_empty()
            || text(&r["ownerSessionId"]).is_empty()
            || text(&r["sourceSessionId"]).is_empty()
            || !request_ids.insert(text(&r["requestId"]))
            || r["revision"]
                .as_u64()
                .is_none_or(|v| v > 9_007_199_254_740_991)
            || !matches!(
                text(&r["delivery"]),
                "pending" | "accepted" | "rejected" | "unknown"
            )
            || !r["sourceCwd"].is_string()
            || !r["createdAt"].is_string()
        {
            bail!("Invalid manager request envelope");
        }
        let digest = text(&r["digest"]);
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            bail!("Invalid manager request digest");
        }
        if r.get("userContent")
            .is_some_and(|v| !v.is_string() || text(v).encode_utf16().count() > 64 * 1024)
        {
            bail!("Invalid manager request content");
        }
        if r.get("intents")
            .is_some_and(|v| !v.as_array().is_some_and(|a| !a.is_empty() && a.len() <= 8))
        {
            bail!("Invalid manager request intents");
        }
        let attempts = r["attempts"]
            .as_array()
            .context("Invalid manager delivery attempts")?;
        let mut seen = BTreeSet::new();
        if attempts.len() > 8 {
            bail!("Too many delivery attempts");
        }
        for a in attempts {
            if text(&a["deliveryId"]).is_empty()
                || !seen.insert(text(&a["deliveryId"]))
                || !matches!(
                    text(&a["status"]),
                    "pending" | "accepted" | "rejected" | "unknown"
                )
            {
                bail!("Invalid delivery attempt");
            }
        }
    }
    Ok(())
}
