//! The queue's data model and its pure rules: parsing a submission, ordering,
//! admission against slots and the resource gate, restart recovery and
//! retention. Nothing here touches the bus, the clock or the disk.
use super::super::host_resources::Thresholds;
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{cmp::Reverse, collections::BTreeMap};

pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";
pub const UNKNOWN: &str = "unknown";

/// Terminal items kept; queued and running items are never evicted.
pub const RETAIN_TERMINAL: usize = 200;
/// `detail` is bounded to this many bytes.
pub const DETAIL_LIMIT: usize = 2000;
pub const DEFAULT_SHELL_TIMEOUT_SEC: u64 = 3600;
pub const MAX_SHELL_TIMEOUT_SEC: u64 = 86_400;
const LABEL_LIMIT: usize = 200;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub job_id: String,
    pub origin: String,
    /// The submitter's own cwd before project-link translation, recorded on
    /// an item forwarded to a peer. Display-only.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub origin_cwd: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SpawnSpec {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    pub cwd: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub permission_mode: String,
    /// Requested context window (tokens), passed to `agents.spawn` as is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Launch with the provider's own approval policy (`skipPermissions`)
    /// instead of `permissionMode`: the job scheduler's
    /// `agents.jobFullAccess`. Only the owner, or a peer paired with the host
    /// credential, may set it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub full_access: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ShellSpec {
    pub command: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    pub timeout_sec: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub priority: i64,
    pub state: String,
    pub source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn: Option<SpawnSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellSpec>,
    pub enqueued_at: i64,
    pub started_at: i64,
    pub finished_at: i64,
    /// Bumped on every change, so clients can order a list against events.
    pub updated_at: i64,
    pub session_id: String,
    pub exit_code: Option<i32>,
    pub detail: String,
    pub wait_reason: String,
    /// The federation credential fingerprint (`Caller::token_id`) that
    /// submitted a `remote` item, so that same peer may cancel it. Persisted,
    /// never sent on the wire.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub submitter: String,
}
impl Item {
    pub fn is_terminal(&self) -> bool {
        !matches!(self.state.as_str(), QUEUED | RUNNING)
    }
    /// The client-facing object (no submitter credential).
    pub fn wire(&self) -> Value {
        let mut value = json!(self);
        if let Some(map) = value.as_object_mut() {
            map.remove("submitter");
        }
        value
    }
}

/// Effective `queue.*` limits.
#[derive(Clone, Debug, PartialEq)]
pub struct QueueConfig {
    pub max_agents: u64,
    pub max_shell: u64,
    pub max_cpu_pressure: f64,
    pub max_memory_pressure: f64,
    pub min_available_memory_mb: u64,
    pub min_free_disk_mb: u64,
}
impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_agents: 4,
            max_shell: 2,
            max_cpu_pressure: 60.0,
            max_memory_pressure: 25.0,
            min_available_memory_mb: 1024,
            min_free_disk_mb: 2048,
        }
    }
}
/// (key, upper bound, integer?) for every `queue.*` key.
const CONFIG_KEYS: &[(&str, f64, bool)] = &[
    ("maxAgents", 64.0, true),
    ("maxShell", 64.0, true),
    ("maxCpuPressure", 100.0, false),
    ("maxMemoryPressure", 100.0, false),
    ("minAvailableMemoryMb", 1_048_576.0, true),
    ("minFreeDiskMb", 1_073_741_824.0, true),
];
impl QueueConfig {
    /// Read the `queue` section of the shared config. A missing or invalid
    /// value (hand-edited YAML) falls back to its default rather than failing.
    pub fn from_section(section: &Value) -> Self {
        let defaults = Self::default();
        let number = |key: &str, max: f64, integer: bool| -> Option<f64> {
            let value = section.get(key)?.as_f64()?;
            (value.is_finite()
                && (0.0..=max).contains(&value)
                && (!integer || value.fract() == 0.0))
                .then_some(value)
        };
        let get = |index: usize| {
            let (key, max, integer) = CONFIG_KEYS[index];
            number(key, max, integer)
        };
        Self {
            max_agents: get(0).map_or(defaults.max_agents, |v| v as u64),
            max_shell: get(1).map_or(defaults.max_shell, |v| v as u64),
            max_cpu_pressure: get(2).unwrap_or(defaults.max_cpu_pressure),
            max_memory_pressure: get(3).unwrap_or(defaults.max_memory_pressure),
            min_available_memory_mb: get(4).map_or(defaults.min_available_memory_mb, |v| v as u64),
            min_free_disk_mb: get(5).map_or(defaults.min_free_disk_mb, |v| v as u64),
        }
    }
    /// Check a `queue.setConfig` patch: an object of known keys only, each a
    /// number in range (integers where the key counts). Returns the patch as
    /// it will be written under `queue`.
    pub fn validate_patch(patch: &Value) -> Result<Value> {
        let map = match patch {
            Value::Object(map) => map,
            Value::Null => return Ok(json!({})),
            _ => bail!("queue.setConfig takes an object of queue limits"),
        };
        let mut out = Map::new();
        for (key, value) in map {
            let &(name, max, integer) = CONFIG_KEYS
                .iter()
                .find(|(name, ..)| *name == key)
                .ok_or_else(|| anyhow!("unknown queue setting {key:?}"))?;
            let number = value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| anyhow!("{name} must be a number"))?;
            if integer && number.fract() != 0.0 {
                bail!("{name} must be a whole number");
            }
            if !(0.0..=max).contains(&number) {
                bail!("{name} must be between 0 and {max}");
            }
            out.insert(
                key.clone(),
                if integer {
                    json!(number as u64)
                } else {
                    json!(number)
                },
            );
        }
        Ok(Value::Object(out))
    }
    pub fn wire(&self) -> Value {
        json!({
            "maxAgents": self.max_agents,
            "maxShell": self.max_shell,
            "maxCpuPressure": self.max_cpu_pressure,
            "maxMemoryPressure": self.max_memory_pressure,
            "minAvailableMemoryMb": self.min_available_memory_mb,
            "minFreeDiskMb": self.min_free_disk_mb,
        })
    }
    pub fn thresholds(&self) -> Thresholds {
        Thresholds {
            max_cpu_pressure: self.max_cpu_pressure,
            max_memory_pressure: self.max_memory_pressure,
            min_available_memory_mb: self.min_available_memory_mb,
            min_free_disk_mb: self.min_free_disk_mb,
        }
    }
    fn slots(&self, kind: &str) -> u64 {
        if kind == "spawn" {
            self.max_agents
        } else {
            self.max_shell
        }
    }
}

const SUBMIT_FIELDS: &[&str] = &[
    "host", "kind", "label", "priority", "spawn", "shell", "source",
];
const SPAWN_FIELDS: &[&str] = &[
    "provider",
    "cwd",
    "prompt",
    "model",
    "effort",
    "permissionMode",
    "contextWindow",
    "fullAccess",
];
const SHELL_FIELDS: &[&str] = &["command", "cwd", "timeoutSec"];
const SOURCE_FIELDS: &[&str] = &["kind", "jobId", "origin", "originCwd"];

fn known_keys(value: &Value, what: &str, known: &[&str]) -> Result<()> {
    if let Some(map) = value.as_object() {
        for key in map.keys() {
            if !known.contains(&key.as_str()) {
                bail!("unknown {what} field {key:?}");
            }
        }
    }
    Ok(())
}
fn text(value: &Value, key: &str, what: &str) -> Result<String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => bail!("{what}.{key} must be a string"),
    }
}
fn absolute(path: &str) -> bool {
    std::path::Path::new(path).is_absolute()
}
fn first_line(text: &str, limit: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    clip(line.unwrap_or(""), limit)
}
/// At most `limit` bytes on a character boundary, with an ellipsis if cut.
pub fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let mut end = limit.saturating_sub(3);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
/// The last `limit` bytes of `text` (trimmed), on a character boundary.
pub fn tail(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.len() <= limit {
        return text.into();
    }
    let mut start = text.len() - limit.saturating_sub(3);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

/// Parse `queue.submit` parameters into a new queued item. Identity, `id`,
/// `submitter` and the final `source.kind` are the caller's to stamp; this
/// only checks shape. `cwd` must be absolute here; whether it exists is the
/// local host's question (`check_local_paths`).
pub fn parse_submit(params: &Value) -> Result<Item> {
    if !params.is_object() {
        bail!("queue.submit takes an object");
    }
    known_keys(params, "queue.submit", SUBMIT_FIELDS)?;
    let kind = text(params, "kind", "queue.submit")?;
    let priority = match params.get("priority") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_i64()
            .filter(|p| (-1_000_000..=1_000_000).contains(p))
            .ok_or_else(|| anyhow!("priority must be an integer"))?,
    };
    let mut label = text(params, "label", "queue.submit")?.trim().to_owned();
    if label.len() > LABEL_LIMIT {
        bail!("label is longer than {LABEL_LIMIT} bytes");
    }
    let present = |key: &str| params.get(key).is_some_and(|v| !v.is_null());
    let mut item = Item {
        kind: kind.clone(),
        priority,
        state: QUEUED.into(),
        ..Default::default()
    };
    match kind.as_str() {
        "spawn" => {
            if present("shell") {
                bail!("a spawn item cannot carry shell");
            }
            let spawn = params
                .get("spawn")
                .filter(|v| v.is_object())
                .ok_or_else(|| anyhow!("a spawn item needs spawn {{cwd, prompt}}"))?;
            known_keys(spawn, "spawn", SPAWN_FIELDS)?;
            let spec = SpawnSpec {
                provider: text(spawn, "provider", "spawn")?.trim().into(),
                cwd: text(spawn, "cwd", "spawn")?.trim().into(),
                prompt: text(spawn, "prompt", "spawn")?,
                model: text(spawn, "model", "spawn")?.trim().into(),
                effort: text(spawn, "effort", "spawn")?.trim().into(),
                permission_mode: text(spawn, "permissionMode", "spawn")?.trim().into(),
                context_window: match spawn.get("contextWindow") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(value.as_u64().filter(|w| *w > 0).ok_or_else(|| {
                        anyhow!("spawn.contextWindow must be a positive integer")
                    })?),
                },
                full_access: match spawn.get("fullAccess") {
                    None | Some(Value::Null) => false,
                    Some(Value::Bool(full)) => *full,
                    Some(_) => bail!("spawn.fullAccess must be a boolean"),
                },
            };
            if spec.cwd.is_empty() || spec.prompt.trim().is_empty() {
                bail!("a spawn item needs spawn.cwd and spawn.prompt");
            }
            if !absolute(&spec.cwd) {
                bail!("spawn.cwd must be an absolute path");
            }
            if label.is_empty() {
                label = first_line(&spec.prompt, 80);
            }
            item.spawn = Some(spec);
        }
        "shell" => {
            if present("spawn") {
                bail!("a shell item cannot carry spawn");
            }
            let shell = params
                .get("shell")
                .filter(|v| v.is_object())
                .ok_or_else(|| anyhow!("a shell item needs shell {{command}}"))?;
            known_keys(shell, "shell", SHELL_FIELDS)?;
            let timeout_sec = match shell.get("timeoutSec") {
                None | Some(Value::Null) => DEFAULT_SHELL_TIMEOUT_SEC,
                Some(value) => value
                    .as_u64()
                    .filter(|t| (1..=MAX_SHELL_TIMEOUT_SEC).contains(t))
                    .ok_or_else(|| {
                        anyhow!("shell.timeoutSec must be 1..={MAX_SHELL_TIMEOUT_SEC}")
                    })?,
            };
            let spec = ShellSpec {
                command: text(shell, "command", "shell")?,
                cwd: text(shell, "cwd", "shell")?.trim().into(),
                timeout_sec,
            };
            if spec.command.trim().is_empty() {
                bail!("a shell item needs shell.command");
            }
            if !spec.cwd.is_empty() && !absolute(&spec.cwd) {
                bail!("shell.cwd must be an absolute path");
            }
            if label.is_empty() {
                label = first_line(&spec.command, 80);
            }
            item.shell = Some(spec);
        }
        _ => bail!("kind must be \"spawn\" or \"shell\""),
    }
    item.label = label;
    if let Some(source) = params.get("source").filter(|v| !v.is_null()) {
        if !source.is_object() {
            bail!("source must be an object");
        }
        known_keys(source, "source", SOURCE_FIELDS)?;
        item.source = Source {
            kind: text(source, "kind", "source")?,
            job_id: text(source, "jobId", "source")?,
            origin: clip(text(source, "origin", "source")?.trim(), LABEL_LIMIT),
            origin_cwd: clip(text(source, "originCwd", "source")?.trim(), 4096),
        };
        if !matches!(
            item.source.kind.as_str(),
            "" | "job" | "prompt" | "agent" | "remote"
        ) {
            bail!("source.kind must be job, prompt, agent or remote");
        }
    }
    if item.source.kind.is_empty() {
        item.source.kind = "prompt".into();
    }
    Ok(item)
}

/// The local host accepts an absolute cwd only if it is an existing directory.
pub fn check_local_paths(item: &Item) -> Result<()> {
    let cwd = match (&item.spawn, &item.shell) {
        (Some(spawn), _) => &spawn.cwd,
        (_, Some(shell)) => &shell.cwd,
        _ => return Ok(()),
    };
    if !cwd.is_empty() && !std::path::Path::new(cwd).is_dir() {
        bail!("cwd {cwd:?} is not a directory on this host");
    }
    Ok(())
}

/// Queue order: higher priority first, then FIFO by enqueue time (id breaks
/// ties so the order is total).
pub fn order(a: &Item, b: &Item) -> std::cmp::Ordering {
    (Reverse(a.priority), a.enqueued_at, &a.id).cmp(&(Reverse(b.priority), b.enqueued_at, &b.id))
}

/// What one admission pass decided.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Items to start now, in queue order.
    pub start: Vec<String>,
    /// The wait reason of every queued item that is not starting (an empty
    /// reason clears a stale one).
    pub waits: BTreeMap<String, String>,
}

/// Decide which queued items start. The gate holds back everything; a kind
/// without a free slot holds back only that kind, so a blocked shell head
/// never stops an agent behind it (and vice versa).
pub fn plan(items: &[Item], config: &QueueConfig, gate: &[String]) -> Plan {
    let mut running: BTreeMap<&str, u64> = BTreeMap::new();
    for item in items.iter().filter(|i| i.state == RUNNING) {
        *running.entry(item.kind.as_str()).or_default() += 1;
    }
    let mut queued: Vec<&Item> = items.iter().filter(|i| i.state == QUEUED).collect();
    queued.sort_by(|a, b| order(a, b));
    let mut plan = Plan::default();
    for item in queued {
        if !gate.is_empty() {
            plan.waits.insert(
                item.id.clone(),
                format!("resource gate closed: {}", gate.join("; ")),
            );
            continue;
        }
        let slots = config.slots(&item.kind);
        let used = running.entry(item.kind.as_str()).or_default();
        if *used < slots {
            *used += 1;
            plan.start.push(item.id.clone());
        } else {
            let noun = if item.kind == "spawn" {
                "agent"
            } else {
                "shell"
            };
            plan.waits.insert(
                item.id.clone(),
                format!("no free {noun} slot ({used}/{slots} running)"),
            );
        }
    }
    plan
}

/// Items persisted as running when the hub stopped have an unknown outcome.
/// They are never re-run. Returns whether anything changed.
pub fn recover(items: &mut [Item], now: i64) -> bool {
    let mut changed = false;
    for item in items.iter_mut().filter(|i| i.state == RUNNING) {
        item.state = UNKNOWN.into();
        item.detail = "hub restarted while running".into();
        item.finished_at = now;
        item.updated_at = now;
        item.wait_reason.clear();
        changed = true;
    }
    changed
}

/// Keep every queued and running item and the newest `RETAIN_TERMINAL`
/// terminal ones (by finish time).
pub fn retain(items: &mut Vec<Item>) {
    let mut terminal: Vec<(i64, i64, String)> = items
        .iter()
        .filter(|i| i.is_terminal())
        .map(|i| (i.finished_at, i.enqueued_at, i.id.clone()))
        .collect();
    if terminal.len() <= RETAIN_TERMINAL {
        return;
    }
    terminal.sort_by(|a, b| b.cmp(a));
    let evicted: std::collections::BTreeSet<String> = terminal
        .into_iter()
        .skip(RETAIN_TERMINAL)
        .map(|(_, _, id)| id)
        .collect();
    items.retain(|i| !evicted.contains(&i.id));
}

/// The listing order: running, then queued in queue order, then terminal
/// items newest first.
pub fn listing(items: &[Item]) -> Vec<&Item> {
    let mut rows: Vec<&Item> = items.iter().collect();
    let rank = |i: &Item| match i.state.as_str() {
        RUNNING => 0,
        QUEUED => 1,
        _ => 2,
    };
    rows.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then_with(|| match rank(a) {
            2 => (b.finished_at, &b.id).cmp(&(a.finished_at, &a.id)),
            _ => order(a, b),
        })
    });
    rows
}
