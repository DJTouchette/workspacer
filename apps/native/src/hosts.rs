//! Hosts and their work queues, as the native Hosts view and the launch host
//! picker show them (`docs/host-work-queue.md`).
//!
//! Every reply is parsed leniently: a field the hub leaves out or sends as
//! `null` is *unknown*, which renders as "—" and never as zero, because a
//! zero pressure or zero free memory would read as a fact. A hub that
//! predates the queue answers its methods with "no provider for …" (or
//! "unknown method"); that is a state of its own ([`Support::Unsupported`]),
//! not an error to retry.
//!
//! Live updates arrive as `host.resources` and `queue.changed` events. They
//! are folded into [`Live`], an overlay over the last snapshot: a resource
//! sample replaces an older one, and an item replaces the listed one unless
//! the list already shows it in a later state (queued → running → finished).
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The local hub, wherever a host is keyed by name.
pub const LOCAL: &str = "local";
/// Items a host's overlay keeps from events between snapshots.
const MAX_LIVE_ITEMS: usize = 256;
/// Finished items a card lists under its queued and running ones.
pub const FINISHED_SHOWN: usize = 8;
/// Shown for any unknown value.
pub const UNKNOWN: &str = "—";

/// Whether an error says the hub does not offer the method at all.
pub fn unsupported(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("no provider for")
        || lower.contains("unknown method")
        || lower.contains("method not found")
        || lower.contains("not offered")
}

fn num(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite())
}

fn int(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| num(value).map(|v| v.round() as i64))
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").trim().to_owned()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `"23%"`, `"—"`.
pub fn pct(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{}%", v.round() as i64),
        None => UNKNOWN.into(),
    }
}

/// A PSI avg10 to one decimal, `"3.1"`, or `"—"`.
pub fn psi(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.1}"),
        None => UNKNOWN.into(),
    }
}

/// `"700 MB"`, `"12.0 GB"`, `"1.2 TB"`, or `"—"`.
pub fn megabytes(mb: Option<f64>) -> String {
    match mb {
        None => UNKNOWN.into(),
        Some(mb) if mb < 1024. => format!("{} MB", mb.round() as i64),
        Some(mb) if mb < 1024. * 1024. => format!("{:.1} GB", mb / 1024.),
        Some(mb) => format!("{:.1} TB", mb / (1024. * 1024.)),
    }
}

/// `"1/4"`, `"—/4"`, `"1/—"`.
fn fraction(used: Option<u64>, of: Option<u64>) -> String {
    let part = |v: Option<u64>| v.map_or_else(|| UNKNOWN.to_owned(), |v| v.to_string());
    format!("{}/{}", part(used), part(of))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gate {
    pub open: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resources {
    /// The hub's own name for itself.
    pub host: String,
    pub sampled_at: Option<i64>,
    pub cores: Option<u64>,
    pub load1: Option<f64>,
    pub cpu_pct: Option<f64>,
    pub cpu_pressure: Option<f64>,
    pub memory_total_mb: Option<f64>,
    pub memory_available_mb: Option<f64>,
    pub memory_pressure: Option<f64>,
    pub io_pressure: Option<f64>,
    pub disk_path: String,
    pub disk_free_mb: Option<f64>,
    pub disk_total_mb: Option<f64>,
    pub agent_slots: Option<u64>,
    pub agents_running: Option<u64>,
    pub shell_slots: Option<u64>,
    pub shell_running: Option<u64>,
    pub queued: Option<u64>,
    /// `None` when the hub did not say.
    pub gate: Option<Gate>,
}

impl Resources {
    /// `None` for anything that is not an object (a peer with no sample).
    pub fn parse(value: &Value) -> Option<Self> {
        if !value.is_object() {
            return None;
        }
        let (cpu, memory, disk, queue) = (
            &value["cpu"],
            &value["memory"],
            &value["disk"],
            &value["queue"],
        );
        let count = |v: &Value| v.as_u64();
        let gate = &queue["gate"];
        Some(Self {
            host: text(value, "host"),
            sampled_at: int(&value["sampledAt"]).filter(|&at| at > 0),
            cores: count(&cpu["cores"]),
            load1: num(&cpu["load1"]),
            cpu_pct: num(&cpu["usagePct"]),
            cpu_pressure: num(&cpu["pressureAvg10"]),
            memory_total_mb: num(&memory["totalMb"]),
            memory_available_mb: num(&memory["availableMb"]),
            memory_pressure: num(&memory["pressureAvg10"]),
            io_pressure: num(&value["io"]["pressureAvg10"]),
            disk_path: text(disk, "path"),
            disk_free_mb: num(&disk["freeMb"]),
            disk_total_mb: num(&disk["totalMb"]),
            agent_slots: count(&queue["agentSlots"]),
            agents_running: count(&queue["agentsRunning"]),
            shell_slots: count(&queue["shellSlots"]),
            shell_running: count(&queue["shellRunning"]),
            queued: count(&queue["queued"]),
            gate: gate["open"].as_bool().map(|open| Gate {
                open,
                reasons: strings(&gate["reasons"]),
            }),
        })
    }

    /// `"CPU 23% · PSI 3.1"`.
    pub fn cpu_line(&self) -> String {
        format!("CPU {} · PSI {}", pct(self.cpu_pct), psi(self.cpu_pressure))
    }

    /// `"Memory PSI 0.0"`.
    pub fn memory_line(&self) -> String {
        format!("Memory PSI {}", psi(self.memory_pressure))
    }

    /// `"IO PSI 0.4"`.
    pub fn io_line(&self) -> String {
        format!("IO PSI {}", psi(self.io_pressure))
    }

    /// `"12.0 GB free"` (of the total when known).
    pub fn memory_free(&self) -> String {
        match self.memory_total_mb {
            Some(_) if self.memory_available_mb.is_some() => format!(
                "{} free of {}",
                megabytes(self.memory_available_mb),
                megabytes(self.memory_total_mb)
            ),
            _ => format!("{} free", megabytes(self.memory_available_mb)),
        }
    }

    /// `"78.1 GB free"` on the hub's state disk.
    pub fn disk_free(&self) -> String {
        format!("{} free", megabytes(self.disk_free_mb))
    }

    /// `"1/4 agents"`.
    pub fn agent_line(&self) -> String {
        format!("{} agents", fraction(self.agents_running, self.agent_slots))
    }

    /// `"0/2 shell"`.
    pub fn shell_line(&self) -> String {
        format!("{} shell", fraction(self.shell_running, self.shell_slots))
    }

    /// `"3 queued"`, `"— queued"`.
    pub fn queued_line(&self) -> String {
        match self.queued {
            Some(n) => format!("{n} queued"),
            None => format!("{UNKNOWN} queued"),
        }
    }

    /// Free agent slots, when both numbers are known.
    pub fn free_agents(&self) -> Option<u64> {
        Some(self.agent_slots?.saturating_sub(self.agents_running?))
    }

    /// `"Gate open"`, `"Gate closed"`, `"Gate unknown"`.
    pub fn gate_label(&self) -> &'static str {
        match &self.gate {
            Some(g) if g.open => "Gate open",
            Some(_) => "Gate closed",
            None => "Gate unknown",
        }
    }

    pub fn gate_reasons(&self) -> &[String] {
        self.gate.as_ref().map_or(&[], |g| &g.reasons)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    Unknown,
    /// A state this client does not know; shown as the hub spelled it.
    Other(String),
}

impl State {
    fn parse(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" | "canceled" => Self::Cancelled,
            "unknown" => Self::Unknown,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "outcome unknown",
            Self::Other(other) => other,
        }
    }

    /// Queued, then running, then any finished state: an item only moves
    /// forward, so the later state of two copies is the newer one.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Queued => 0,
            Self::Running => 1,
            _ => 2,
        }
    }

    pub fn finished(&self) -> bool {
        self.rank() == 2
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    /// `spawn` or `shell`.
    pub kind: String,
    pub label: String,
    pub priority: i64,
    pub state: State,
    /// `job`, `prompt`, `agent` or `remote`.
    pub source: String,
    pub origin: String,
    pub job_id: String,
    pub enqueued_at: Option<i64>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub session_id: String,
    pub exit_code: Option<i64>,
    pub detail: String,
    pub wait_reason: String,
    pub provider: String,
    pub cwd: String,
    pub prompt: String,
    pub command: String,
}

impl Item {
    pub fn parse(value: &Value) -> Option<Self> {
        let id = text(value, "id");
        if id.is_empty() {
            return None;
        }
        let at = |key: &str| int(&value[key]).filter(|&at| at > 0);
        let (spawn, shell, source) = (&value["spawn"], &value["shell"], &value["source"]);
        let kind = match text(value, "kind") {
            kind if !kind.is_empty() => kind,
            _ if shell.is_object() => "shell".into(),
            _ => "spawn".into(),
        };
        let cwd = if kind == "shell" {
            text(shell, "cwd")
        } else {
            text(spawn, "cwd")
        };
        Some(Self {
            id,
            kind,
            label: text(value, "label"),
            priority: int(&value["priority"]).unwrap_or(0),
            state: State::parse(value["state"].as_str().unwrap_or("")),
            source: text(source, "kind"),
            origin: text(source, "origin"),
            job_id: text(source, "jobId"),
            enqueued_at: at("enqueuedAt"),
            started_at: at("startedAt"),
            finished_at: at("finishedAt"),
            session_id: text(value, "sessionId"),
            exit_code: value["exitCode"].as_i64(),
            detail: text(value, "detail"),
            wait_reason: text(value, "waitReason"),
            provider: text(spawn, "provider"),
            cwd,
            prompt: text(spawn, "prompt"),
            command: text(shell, "command"),
        })
    }

    /// The label, else the first line of the prompt or the command.
    pub fn title(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        let first = |s: &str| {
            s.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_owned)
        };
        if self.kind == "shell" {
            first(&self.command)
                .map(|c| format!("$ {c}"))
                .unwrap_or_else(|| "shell".into())
        } else {
            first(&self.prompt).unwrap_or_else(|| format!("{} agent", self.provider_name()))
        }
    }

    fn provider_name(&self) -> &str {
        match self.provider.as_str() {
            "claude" => "Claude",
            "codex" => "Codex",
            "" => "Agent",
            other => other,
        }
    }

    /// `"Claude in /work/api"`, `"$ make test in /work/api"`.
    pub fn what(&self) -> String {
        let base = if self.kind == "shell" {
            format!("shell · $ {}", self.command)
        } else {
            self.provider_name().to_owned()
        };
        if self.cwd.is_empty() {
            base
        } else {
            format!("{base} in {}", self.cwd)
        }
    }

    /// Kind, priority (when not 0) and where it came from: `"spawn · p2 · job"`.
    pub fn tags(&self) -> String {
        let mut tags = vec![self.kind.clone()];
        if self.priority != 0 {
            tags.push(format!("p{}", self.priority));
        }
        match (self.source.as_str(), self.origin.as_str()) {
            ("", _) => {}
            ("remote", origin) if !origin.is_empty() => tags.push(format!("from {origin}")),
            (source, _) => tags.push(source.to_owned()),
        }
        tags.join(" · ")
    }

    /// When it entered its current state: `"queued 5m ago"`,
    /// `"started 2m ago"`, `"finished 1h ago"`.
    pub fn when(&self, now: i64) -> String {
        let (verb, at) = match self.state {
            State::Queued => ("queued", self.enqueued_at),
            State::Running => ("started", self.started_at.or(self.enqueued_at)),
            _ => (
                "finished",
                self.finished_at.or(self.started_at).or(self.enqueued_at),
            ),
        };
        match at {
            Some(at) => format!("{verb} {}", crate::jobs::since(at, now)),
            None => verb.to_owned(),
        }
    }

    /// Only queued work can be cancelled; the hub refuses running items.
    pub fn cancellable(&self) -> bool {
        self.state == State::Queued
    }

    /// The line under the row: why it waits, how it ended, or its session.
    pub fn note(&self) -> String {
        match self.state {
            State::Queued if !self.wait_reason.is_empty() => {
                format!("Waiting: {}", self.wait_reason)
            }
            State::Queued => String::new(),
            State::Running if !self.session_id.is_empty() => format!("Session {}", self.session_id),
            _ => {
                let first = self
                    .detail
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("");
                let exit = self
                    .exit_code
                    .filter(|&code| code != 0)
                    .map(|code| format!("exit {code}"));
                [exit, (!first.is_empty()).then(|| first.to_owned())]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ")
            }
        }
    }
}

/// Queued and running first (running, then by priority and age as the hub
/// admits them), then the newest finished items, at most `finished`.
pub fn ordered(mut items: Vec<Item>, finished: usize) -> Vec<Item> {
    items.sort_by(|a, b| {
        let (a_done, b_done) = (a.state.finished(), b.state.finished());
        a_done.cmp(&b_done).then_with(|| {
            if a_done {
                b.finished_at.cmp(&a.finished_at)
            } else {
                (b.state == State::Running)
                    .cmp(&(a.state == State::Running))
                    .then(b.priority.cmp(&a.priority))
                    .then(a.enqueued_at.cmp(&b.enqueued_at))
            }
        })
    });
    let open = items.iter().filter(|i| !i.state.finished()).count();
    items.truncate(open + finished);
    items
}

fn parse_items(value: &Value) -> Vec<Item> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Item::parse)
        .collect()
}

/// One host's rows from `queue.list`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostQueue {
    /// The hub's own name (local) or the peer's.
    pub host: String,
    pub ok: bool,
    pub error: String,
    pub items: Vec<Item>,
}

/// `queue.list {includePeers: true}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueList {
    pub local: HostQueue,
    pub peers: Vec<HostQueue>,
}

pub fn parse_list(value: &Value) -> QueueList {
    QueueList {
        local: HostQueue {
            host: text(value, "host"),
            ok: true,
            error: String::new(),
            items: parse_items(&value["items"]),
        },
        peers: value["peers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|peer| {
                let host = text(peer, "peer");
                (!host.is_empty()).then(|| HostQueue {
                    ok: peer["ok"] != false,
                    error: text(peer, "error"),
                    items: parse_items(&peer["items"]),
                    host,
                })
            })
            .collect(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderState {
    pub provider: String,
    pub found: bool,
    pub authenticated: bool,
}

/// One entry of `queue.targets`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    /// `local` or the peer's name.
    pub host: String,
    pub local: bool,
    pub connected: bool,
    pub dispatch: bool,
    pub resources: Option<Resources>,
    pub providers: Vec<ProviderState>,
    pub cwds: Vec<String>,
    pub eligible: bool,
    pub score: Option<f64>,
    pub reasons: Vec<String>,
}

impl Target {
    fn parse(value: &Value) -> Option<Self> {
        let host = text(value, "host");
        if host.is_empty() {
            return None;
        }
        Some(Self {
            local: value["local"] == true || host == LOCAL,
            connected: value["connected"] == true,
            dispatch: value["dispatch"] == true,
            resources: Resources::parse(&value["resources"]),
            providers: value["providers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    let provider = text(p, "provider");
                    (!provider.is_empty()).then(|| ProviderState {
                        provider,
                        found: p["found"] == true,
                        authenticated: p["authenticated"] == true,
                    })
                })
                .collect(),
            cwds: strings(&value["cwds"]),
            eligible: value["eligible"] == true,
            score: num(&value["score"]),
            reasons: strings(&value["reasons"]),
            host,
        })
    }

    /// `"This machine"` or the peer's name.
    pub fn name(&self) -> &str {
        if self.local {
            "This machine"
        } else {
            &self.host
        }
    }
}

/// `queue.targets`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Targets {
    pub targets: Vec<Target>,
    /// The host placement would choose now.
    pub best: Option<String>,
}

impl Targets {
    pub fn peers(&self) -> impl Iterator<Item = &Target> {
        self.targets.iter().filter(|t| !t.local)
    }

    pub fn find(&self, host: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.host == host)
    }
}

pub fn parse_targets(value: &Value) -> Option<Targets> {
    let targets = value["targets"].as_array()?;
    Some(Targets {
        targets: targets.iter().filter_map(Target::parse).collect(),
        best: value["best"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    })
}

/// The six `queue.*` limits, wire key → (label, unit, whole number).
pub const LIMITS: [(&str, &str, &str); 6] = [
    ("maxAgents", "Agent slots", ""),
    ("maxShell", "Shell slots", ""),
    ("maxCpuPressure", "Max CPU pressure", "%"),
    ("maxMemoryPressure", "Max memory pressure", "%"),
    ("minAvailableMemoryMb", "Min available memory", "MB"),
    ("minFreeDiskMb", "Min free disk", "MB"),
];

/// `queue.config`: the effective limit for each key, `None` when unsent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limits {
    pub values: BTreeMap<&'static str, Option<f64>>,
}

impl Limits {
    pub fn parse(value: &Value) -> Option<Self> {
        value.is_object().then(|| Self {
            values: LIMITS
                .iter()
                .map(|(key, ..)| (*key, num(&value[*key])))
                .collect(),
        })
    }

    pub fn get(&self, key: &str) -> Option<f64> {
        self.values.get(key).copied().flatten()
    }

    /// The value as a field shows it: `"60"`, `"1024"`, or empty.
    pub fn text(&self, key: &str) -> String {
        self.get(key).map(format_limit).unwrap_or_default()
    }
}

fn format_limit(value: f64) -> String {
    if value.fract() == 0. {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// The `queue.setConfig` patch for the fields that differ from `current`:
/// each must be a non-negative number, slots and MB whole. Empty fields are
/// left alone. `None` when nothing changed.
pub fn config_patch(
    current: &Limits,
    fields: &[(&'static str, String)],
) -> Result<Option<Value>, String> {
    let mut patch = serde_json::Map::new();
    for (key, raw) in fields {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let label = LIMITS
            .iter()
            .find(|(k, ..)| k == key)
            .map_or(*key, |(_, label, _)| *label);
        let whole = !key.ends_with("Pressure");
        let value: f64 = raw
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite() && *v >= 0.)
            .ok_or_else(|| format!("{label} must be a number of 0 or more."))?;
        if whole && value.fract() != 0. {
            return Err(format!("{label} must be a whole number."));
        }
        if key.ends_with("Pressure") && value > 100. {
            return Err(format!("{label} is a percentage (0–100)."));
        }
        if current.get(key) == Some(value) {
            continue;
        }
        patch.insert(
            (*key).to_owned(),
            if whole {
                json!(value as i64)
            } else {
                json!(value)
            },
        );
    }
    Ok((!patch.is_empty()).then_some(Value::Object(patch)))
}

/// What the Hosts view read: one part per method, each with its own error.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub resources: Result<Resources, String>,
    pub list: Result<QueueList, String>,
    pub targets: Result<Targets, String>,
    pub limits: Result<Limits, String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Support {
    #[default]
    Supported,
    /// The hub predates the work queue.
    Unsupported,
}

impl Snapshot {
    /// The combined reply the Hosts read builds: `{resources, list, targets,
    /// config}`, each the method's result or `{"error": "..."}`.
    pub fn parse(value: &Value) -> Self {
        fn part<T>(value: &Value, parse: impl Fn(&Value) -> Option<T>) -> Result<T, String> {
            if let Some(error) = value.get("error").and_then(Value::as_str)
                && value.as_object().is_some_and(|m| m.len() == 1)
            {
                return Err(error.to_owned());
            }
            parse(value).ok_or_else(|| "The hub returned an unexpected reply.".to_owned())
        }
        Self {
            resources: part(&value["resources"], Resources::parse),
            list: part(&value["list"], |v| v.is_object().then(|| parse_list(v))),
            targets: part(&value["targets"], parse_targets),
            limits: part(&value["config"], Limits::parse),
        }
    }

    /// Unsupported when the queue itself (`queue.list`) and the resources
    /// are both missing: a hub with neither has no queue to show.
    pub fn support(&self) -> Support {
        match (&self.list, &self.resources) {
            (Err(a), Err(b)) if unsupported(a) && unsupported(b) => Support::Unsupported,
            _ => Support::Supported,
        }
    }
}

/// One card of the Hosts view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    /// `local` for this hub, else the peer's name (as `queue.cancel` and
    /// `queue.submit` take it).
    pub host: String,
    /// What the card is titled.
    pub name: String,
    pub local: bool,
    /// Peers only: from `queue.targets`; `None` when it did not list them.
    pub connected: Option<bool>,
    pub dispatch: Option<bool>,
    pub resources: Option<Resources>,
    /// Queued and running work, then recent finished items.
    pub items: Vec<Item>,
    /// Why this host's queue could not be listed.
    pub error: String,
}

/// Local first, then each peer named by `queue.list` or `queue.targets`, in
/// that order, with the live overlay folded in.
pub fn cards(snapshot: &Snapshot, live: &Live) -> Vec<Card> {
    let targets = snapshot.targets.as_ref().ok();
    let list = snapshot.list.as_ref().ok();
    let local_name = snapshot
        .resources
        .as_ref()
        .ok()
        .map(|r| r.host.clone())
        .filter(|h| !h.is_empty())
        .or_else(|| list.map(|l| l.local.host.clone()).filter(|h| !h.is_empty()));
    let mut local = Card {
        host: LOCAL.into(),
        name: local_name
            .map(|n| format!("This machine · {n}"))
            .unwrap_or_else(|| "This machine".into()),
        local: true,
        resources: newer(
            snapshot.resources.as_ref().ok().cloned(),
            live.resources.get(LOCAL),
        ),
        items: merge(
            list.map(|l| l.local.items.clone()).unwrap_or_default(),
            live.items.get(LOCAL),
        ),
        error: match &snapshot.list {
            Err(e) => e.clone(),
            Ok(_) => String::new(),
        },
        ..Default::default()
    };
    if local.resources.is_none()
        && let Some(target) = targets.and_then(|t| t.targets.iter().find(|t| t.local))
    {
        local.resources = target.resources.clone();
    }
    let mut cards = vec![local];
    let mut names: Vec<String> = Vec::new();
    for name in list
        .into_iter()
        .flat_map(|l| l.peers.iter().map(|p| p.host.clone()))
        .chain(
            targets
                .into_iter()
                .flat_map(|t| t.peers().map(|p| p.host.clone())),
        )
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    for name in names {
        let target = targets.and_then(|t| t.find(&name)).filter(|t| !t.local);
        let queue = list.and_then(|l| l.peers.iter().find(|p| p.host == name));
        cards.push(Card {
            connected: target.map(|t| t.connected),
            dispatch: target.map(|t| t.dispatch),
            resources: newer(
                target.and_then(|t| t.resources.clone()),
                live.resources.get(&name),
            ),
            items: merge(
                queue.map(|q| q.items.clone()).unwrap_or_default(),
                live.items.get(&name),
            ),
            error: queue
                .filter(|q| !q.ok)
                .map(|q| {
                    if q.error.is_empty() {
                        "This peer did not list its queue.".into()
                    } else {
                        q.error.clone()
                    }
                })
                .unwrap_or_default(),
            name: name.clone(),
            host: name,
            local: false,
        });
    }
    cards
}

fn newer(listed: Option<Resources>, live: Option<&Resources>) -> Option<Resources> {
    match (listed, live) {
        (Some(listed), Some(live)) if live.sampled_at > listed.sampled_at => Some(live.clone()),
        (None, Some(live)) => Some(live.clone()),
        (listed, _) => listed,
    }
}

/// The listed items with each live copy that is at least as far along, plus
/// live items the list does not have yet.
pub fn merge(mut listed: Vec<Item>, live: Option<&BTreeMap<String, Item>>) -> Vec<Item> {
    for item in live.into_iter().flat_map(BTreeMap::values) {
        match listed.iter_mut().find(|i| i.id == item.id) {
            Some(row) if item.state.rank() > row.state.rank() => *row = item.clone(),
            Some(_) => {}
            None => listed.push(item.clone()),
        }
    }
    ordered(listed, FINISHED_SHOWN)
}

/// Events since the last snapshot, keyed by host (`local` or the peer).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub resources: BTreeMap<String, Resources>,
    pub items: BTreeMap<String, BTreeMap<String, Item>>,
}

impl Live {
    /// Fold a `host.resources` or `queue.changed` event. `hub` is the peer a
    /// forwarded event came from; `None` (or empty) is this hub. Returns
    /// whether anything changed.
    pub fn apply(&mut self, topic: &str, hub: Option<&str>, data: &Value) -> bool {
        let host = hub
            .filter(|h| !h.is_empty())
            .or_else(|| data["hub"].as_str().filter(|h| !h.is_empty()))
            .unwrap_or(LOCAL)
            .to_owned();
        match topic {
            "host.resources" => {
                let Some(sample) = Resources::parse(data) else {
                    return false;
                };
                if self
                    .resources
                    .get(&host)
                    .is_some_and(|held| held.sampled_at > sample.sampled_at)
                {
                    return false;
                }
                self.resources.insert(host, sample);
                true
            }
            "queue.changed" => {
                let Some(item) = Item::parse(&data["item"]) else {
                    return false;
                };
                let items = self.items.entry(host).or_default();
                if items
                    .get(&item.id)
                    .is_some_and(|held| held.state.rank() > item.state.rank())
                {
                    return false;
                }
                items.insert(item.id.clone(), item);
                if items.len() > MAX_LIVE_ITEMS {
                    // Forget the oldest finished item, else the oldest.
                    let drop = items
                        .values()
                        .filter(|i| i.state.finished())
                        .min_by_key(|i| i.finished_at)
                        .or_else(|| items.values().min_by_key(|i| i.enqueued_at))
                        .map(|i| i.id.clone());
                    if let Some(id) = drop {
                        items.remove(&id);
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Drop what a fresh snapshot already shows at least as far along.
    pub fn settle(&mut self, snapshot: &Snapshot) {
        if let Ok(list) = &snapshot.list {
            for queue in std::iter::once(&list.local).chain(&list.peers) {
                let key = if std::ptr::eq(queue, &list.local) {
                    LOCAL
                } else {
                    queue.host.as_str()
                };
                if let Some(items) = self.items.get_mut(key) {
                    items.retain(|id, item| {
                        !queue
                            .items
                            .iter()
                            .any(|row| &row.id == id && row.state.rank() >= item.state.rank())
                    });
                }
            }
        }
        if let Ok(resources) = &snapshot.resources
            && self
                .resources
                .get(LOCAL)
                .is_some_and(|held| held.sampled_at <= resources.sampled_at)
        {
            self.resources.remove(LOCAL);
        }
    }
}

/// Where a launch runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HostChoice {
    /// This machine, through today's direct `agents.spawn` unless queued.
    #[default]
    Local,
    Peer(String),
    /// Placement picks (`host: "any"`).
    Best,
}

impl HostChoice {
    /// `host` as `queue.submit` takes it.
    pub fn wire(&self) -> &str {
        match self {
            Self::Local => LOCAL,
            Self::Peer(peer) => peer,
            Self::Best => "any",
        }
    }
}

/// One option of the launch host picker.
#[derive(Clone, Debug, PartialEq)]
pub struct HostOption {
    pub choice: HostChoice,
    pub label: String,
    /// A short state line: slots and gate, or why it cannot take the work.
    pub detail: String,
    pub enabled: bool,
    pub reasons: Vec<String>,
}

/// This machine, each peer `queue.targets` lists, and Best fit (enabled
/// while any target is eligible).
pub fn host_options(targets: &Targets) -> Vec<HostOption> {
    let state = |t: &Target| {
        t.resources
            .as_ref()
            .map(|r| format!("{} · {}", r.agent_line(), r.gate_label().to_lowercase()))
            .unwrap_or_default()
    };
    let local = targets.targets.iter().find(|t| t.local);
    let mut options = vec![HostOption {
        choice: HostChoice::Local,
        label: "This machine".into(),
        detail: local.map(state).unwrap_or_default(),
        enabled: true,
        reasons: Vec::new(),
    }];
    for peer in targets.peers() {
        let mut reasons = peer.reasons.clone();
        if reasons.is_empty() && !peer.eligible {
            if !peer.connected {
                reasons.push("not connected".into());
            } else if !peer.dispatch {
                reasons.push("dispatch is off for this peer".into());
            }
        }
        options.push(HostOption {
            choice: HostChoice::Peer(peer.host.clone()),
            label: peer.host.clone(),
            detail: if peer.eligible {
                state(peer)
            } else {
                "unavailable".into()
            },
            enabled: peer.eligible,
            reasons: if peer.eligible { Vec::new() } else { reasons },
        });
    }
    let any = targets.targets.iter().any(|t| t.eligible);
    options.push(HostOption {
        choice: HostChoice::Best,
        label: "Best fit".into(),
        detail: match (&targets.best, any) {
            (_, false) => "no host can take it now".into(),
            (Some(best), true) if best == LOCAL => "now: this machine".into(),
            (Some(best), true) => format!("now: {best}"),
            (None, true) => String::new(),
        },
        enabled: any,
        reasons: Vec::new(),
    });
    options
}

/// A launch that goes through the queue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnSubmit {
    pub host: HostChoice,
    pub provider: String,
    pub cwd: String,
    pub prompt: String,
    pub model: String,
    pub effort: String,
    pub permission_mode: String,
    pub label: String,
}

impl SpawnSubmit {
    /// `queue.submit` params, `kind: "spawn"`, source `prompt`.
    pub fn params(&self) -> Result<Value, String> {
        if !matches!(self.provider.as_str(), "claude" | "codex") {
            return Err("Choose Claude or Codex.".into());
        }
        let cwd = self.cwd.trim();
        if !crate::launch::absolute_directory(cwd) {
            return Err("Choose a project folder on that host.".into());
        }
        if self.prompt.trim().is_empty() {
            return Err("A queued agent needs a task: write what it should do.".into());
        }
        if self.prompt.len() > 65536 {
            return Err("The task must be at most 64 KiB.".into());
        }
        let mut spawn = json!({
            "provider": self.provider,
            "cwd": cwd,
            "prompt": self.prompt.trim(),
        });
        for (key, value) in [
            ("model", &self.model),
            ("effort", &self.effort),
            ("permissionMode", &self.permission_mode),
        ] {
            if !value.trim().is_empty() {
                spawn[key] = json!(value.trim());
            }
        }
        let mut params = json!({
            "host": self.host.wire(),
            "kind": "spawn",
            "spawn": spawn,
            "source": {"kind": "prompt"},
        });
        if !self.label.trim().is_empty() {
            params["label"] = json!(self.label.trim());
        }
        Ok(params)
    }
}

/// `queue.submit`'s answer: where it landed and the item.
pub fn parse_submitted(value: &Value) -> Option<(String, Item)> {
    let item = Item::parse(&value["item"])?;
    let host = value["host"].as_str().unwrap_or(LOCAL).trim();
    Some((if host.is_empty() { LOCAL } else { host }.to_owned(), item))
}

/// `"Queued on gpu-box as q_01J…"`, `"Started on this machine …"`.
pub fn submitted_line(host: &str, item: &Item) -> String {
    let place = if host == LOCAL || host.is_empty() {
        "this machine".to_owned()
    } else {
        host.to_owned()
    };
    let verb = match item.state {
        State::Running => "Started",
        State::Queued => "Queued",
        _ => "Submitted",
    };
    format!("{verb} on {place} as {}.", item.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "host": "studio",
            "sampledAt": 1_762_300_000_000_i64,
            "cpu":    { "cores": 8, "load1": 1.2, "usagePct": 23.5, "pressureAvg10": 3.1 },
            "memory": { "totalMb": 31800, "availableMb": 12288, "pressureAvg10": 0.0 },
            "io":     { "pressureAvg10": 0.4 },
            "disk":   { "path": "/home/me/.config/workspacer-hub", "freeMb": 80000, "totalMb": 500000 },
            "queue":  {
                "agentSlots": 4, "agentsRunning": 1,
                "shellSlots": 2, "shellRunning": 0,
                "queued": 3,
                "gate": { "open": false, "reasons": ["memory pressure 34.0 >= 25.0"] }
            }
        })
    }

    #[test]
    fn resources_render_known_and_unknown_values() {
        let r = Resources::parse(&sample()).unwrap();
        assert_eq!(r.cpu_line(), "CPU 24% · PSI 3.1");
        assert_eq!(r.memory_line(), "Memory PSI 0.0", "zero is a reading");
        assert_eq!(r.io_line(), "IO PSI 0.4");
        assert_eq!(r.memory_free(), "12.0 GB free of 31.1 GB");
        assert_eq!(r.disk_free(), "78.1 GB free");
        assert_eq!(r.agent_line(), "1/4 agents");
        assert_eq!(r.shell_line(), "0/2 shell");
        assert_eq!(r.queued_line(), "3 queued");
        assert_eq!(r.free_agents(), Some(3));
        assert_eq!(r.gate_label(), "Gate closed");
        assert_eq!(r.gate_reasons(), ["memory pressure 34.0 >= 25.0"]);

        // macOS: no PSI. Null is unknown, never zero.
        let mut mac = sample();
        mac["cpu"]["pressureAvg10"] = Value::Null;
        mac["memory"] = json!({"totalMb": 16384});
        mac["io"] = json!({"pressureAvg10": null});
        mac["queue"] = json!({});
        let r = Resources::parse(&mac).unwrap();
        assert_eq!(r.cpu_line(), "CPU 24% · PSI —");
        assert_eq!(r.memory_line(), "Memory PSI —");
        assert_eq!(r.io_line(), "IO PSI —");
        assert_eq!(r.memory_free(), "— free");
        assert_eq!(r.agent_line(), "—/— agents");
        assert_eq!(r.queued_line(), "— queued");
        assert_eq!(r.free_agents(), None);
        assert_eq!(r.gate_label(), "Gate unknown");
        assert!(r.gate_reasons().is_empty());

        let bare = Resources::parse(&json!({})).unwrap();
        assert_eq!(bare.cpu_line(), "CPU — · PSI —");
        assert_eq!(bare.disk_free(), "— free");
        assert!(Resources::parse(&Value::Null).is_none());
        assert_eq!(megabytes(Some(700.)), "700 MB");
        assert_eq!(megabytes(Some(2. * 1024. * 1024.)), "2.0 TB");
    }

    fn item(id: &str, state: &str, priority: i64, enqueued: i64) -> Value {
        json!({"id": id, "kind": "spawn", "label": "", "priority": priority, "state": state,
            "source": {"kind": "job", "jobId": "j1", "origin": ""},
            "spawn": {"provider": "claude", "cwd": "/work/api", "prompt": "Triage the failures\nmore"},
            "enqueuedAt": enqueued, "startedAt": 0, "finishedAt": 0, "exitCode": null,
            "detail": "", "waitReason": "no free agent slot"})
    }

    #[test]
    fn items_parse_label_and_order() {
        let list = parse_list(&json!({
            "host": "studio",
            "items": [
                item("a", "queued", 0, 10),
                item("b", "queued", 5, 20),
                item("c", "running", 0, 5),
                {"id": "d", "kind": "shell", "state": "failed", "label": "nightly",
                 "shell": {"command": "make test", "cwd": "/work/api"},
                 "enqueuedAt": 1, "startedAt": 2, "finishedAt": 3, "exitCode": 2,
                 "detail": "\nFAIL cart\n"},
                {"id": "", "state": "queued"},
                {"id": "e", "state": "weird"}
            ],
            "peers": [
                {"peer": "gpu-box", "ok": true, "items": [item("p1", "queued", 0, 1)]},
                {"peer": "laptop", "ok": false, "error": "timed out", "items": []},
                {"ok": true}
            ]
        }));
        assert_eq!(list.local.host, "studio");
        assert_eq!(list.local.items.len(), 5);
        assert_eq!(list.peers.len(), 2);
        assert!(!list.peers[1].ok);
        let ids: Vec<_> = ordered(list.local.items.clone(), 8)
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids, ["c", "b", "a", "d", "e"].map(String::from));

        let queued = &list.local.items[0];
        assert_eq!(queued.title(), "Triage the failures");
        assert_eq!(queued.what(), "Claude in /work/api");
        assert_eq!(queued.tags(), "spawn · job");
        assert!(queued.cancellable());
        assert_eq!(queued.note(), "Waiting: no free agent slot");
        assert_eq!(queued.when(10 + 5 * 60_000), "queued 5m ago");
        assert_eq!(list.local.items[1].tags(), "spawn · p5 · job");

        let failed = &list.local.items[3];
        assert_eq!(failed.title(), "nightly");
        assert_eq!(failed.what(), "shell · $ make test in /work/api");
        assert_eq!(failed.state.label(), "failed");
        assert!(!failed.cancellable());
        assert_eq!(failed.note(), "exit 2 · FAIL cart");
        assert_eq!(list.local.items[4].state.label(), "weird");
        assert_eq!(
            State::parse("unknown").label(),
            "outcome unknown",
            "never re-run, so say so"
        );
    }

    #[test]
    fn targets_and_host_options() {
        let targets = parse_targets(&json!({
            "targets": [
                {"host": "local", "local": true, "connected": true, "dispatch": true,
                 "resources": sample(), "providers": [{"provider": "claude", "found": true, "authenticated": true}],
                 "cwds": [], "eligible": true, "score": 3.5, "reasons": []},
                {"host": "gpu-box", "local": false, "connected": true, "dispatch": true,
                 "resources": null, "providers": [], "cwds": ["/srv/api", "", "/srv/web"],
                 "eligible": true, "score": 1, "reasons": []},
                {"host": "laptop", "connected": false, "dispatch": true, "eligible": false,
                 "reasons": ["not connected"]},
                {"host": "nas", "connected": true, "dispatch": false, "eligible": false}
            ],
            "best": "gpu-box"
        }))
        .unwrap();
        assert_eq!(targets.targets.len(), 4);
        assert_eq!(
            targets.find("gpu-box").unwrap().cwds,
            ["/srv/api", "/srv/web"]
        );
        assert!(targets.find("gpu-box").unwrap().resources.is_none());
        let options = host_options(&targets);
        let labels: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(
            labels,
            ["This machine", "gpu-box", "laptop", "nas", "Best fit"]
        );
        assert_eq!(options[0].detail, "1/4 agents · gate closed");
        assert!(options[1].enabled);
        assert!(!options[2].enabled);
        assert_eq!(options[2].reasons, ["not connected"]);
        assert_eq!(options[3].reasons, ["dispatch is off for this peer"]);
        assert_eq!(options[4].detail, "now: gpu-box");
        assert_eq!(options[4].choice.wire(), "any");
        assert_eq!(options[1].choice.wire(), "gpu-box");
        assert!(parse_targets(&json!({})).is_none());
        let none = host_options(&Targets::default());
        assert_eq!(none.len(), 2);
        assert!(!none[1].enabled);
    }

    #[test]
    fn limits_parse_and_patch() {
        let limits = Limits::parse(&json!({"maxAgents": 4, "maxShell": 2, "maxCpuPressure": 60,
            "maxMemoryPressure": 25.5, "minAvailableMemoryMb": 1024}))
        .unwrap();
        assert_eq!(limits.text("maxAgents"), "4");
        assert_eq!(limits.text("maxMemoryPressure"), "25.5");
        assert_eq!(limits.text("minFreeDiskMb"), "", "unsent is blank, not 0");
        let field = |k: &'static str, v: &str| (k, v.to_owned());
        assert_eq!(
            config_patch(&limits, &[field("maxAgents", "4"), field("maxShell", " ")]).unwrap(),
            None
        );
        assert_eq!(
            config_patch(
                &limits,
                &[
                    field("maxAgents", "6"),
                    field("maxCpuPressure", "42.5"),
                    field("minFreeDiskMb", "4096")
                ]
            )
            .unwrap(),
            Some(json!({"maxAgents": 6, "maxCpuPressure": 42.5, "minFreeDiskMb": 4096}))
        );
        assert!(
            config_patch(&limits, &[field("maxAgents", "1.5")])
                .unwrap_err()
                .contains("whole")
        );
        assert!(config_patch(&limits, &[field("maxShell", "-1")]).is_err());
        assert!(config_patch(&limits, &[field("maxCpuPressure", "150")]).is_err());
        assert!(Limits::parse(&Value::Null).is_none());
    }

    fn snapshot_value() -> Value {
        json!({
            "resources": sample(),
            "list": {"host": "studio", "items": [item("a", "queued", 0, 10)],
                     "peers": [{"peer": "gpu-box", "ok": true, "items": []},
                               {"peer": "laptop", "ok": false, "error": "timed out"}]},
            "targets": {"targets": [
                {"host": "local", "local": true, "connected": true, "eligible": true},
                {"host": "gpu-box", "connected": true, "dispatch": true, "eligible": true,
                 "resources": {"host": "gpu-box", "sampledAt": 5, "cpu": {"usagePct": 90}}},
                {"host": "nas", "connected": false, "dispatch": false, "eligible": false}
            ], "best": "local"},
            "config": {"maxAgents": 4}
        })
    }

    #[test]
    fn snapshot_builds_cards_local_first() {
        let snapshot = Snapshot::parse(&snapshot_value());
        assert_eq!(snapshot.support(), Support::Supported);
        let cards = cards(&snapshot, &Live::default());
        let names: Vec<_> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["This machine · studio", "gpu-box", "laptop", "nas"]);
        assert_eq!(cards[0].host, "local");
        assert_eq!(cards[0].items.len(), 1);
        assert_eq!(cards[1].connected, Some(true));
        assert_eq!(
            cards[1].resources.as_ref().unwrap().cpu_line(),
            "CPU 90% · PSI —"
        );
        assert_eq!(cards[2].error, "timed out");
        assert_eq!(cards[2].connected, None, "queue.targets did not list it");
        assert_eq!(cards[3].dispatch, Some(false));
        assert_eq!(snapshot.limits.as_ref().unwrap().text("maxAgents"), "4");
    }

    #[test]
    fn older_hubs_are_unsupported_not_failed() {
        let missing = |m: &str| json!({"error": format!("no provider for {m}")});
        let old = Snapshot::parse(&json!({
            "resources": missing("host.resources"),
            "list": missing("queue.list"),
            "targets": missing("queue.targets"),
            "config": missing("queue.config"),
        }));
        assert_eq!(old.support(), Support::Unsupported);
        // Only the list failed for another reason: still a supported hub.
        let mut flaky = snapshot_value();
        flaky["list"] = json!({"error": "timed out"});
        let flaky = Snapshot::parse(&flaky);
        assert_eq!(flaky.support(), Support::Supported);
        let cards = cards(&flaky, &Live::default());
        assert_eq!(cards[0].error, "timed out");
        assert_eq!(cards.len(), 3, "peers still come from targets");
        assert!(unsupported("unknown method: queue.list"));
        assert!(!unsupported("permission denied"));
        // A missing part (not even an error) is an unexpected reply.
        assert!(Snapshot::parse(&json!({})).resources.is_err());
    }

    #[test]
    fn live_events_overlay_the_snapshot() {
        let snapshot = Snapshot::parse(&snapshot_value());
        let mut live = Live::default();
        // A newer local sample replaces the listed one; an older does not.
        let mut newer_sample = sample();
        newer_sample["sampledAt"] = json!(1_762_300_005_000_i64);
        newer_sample["queue"]["gate"] = json!({"open": true, "reasons": []});
        assert!(live.apply("host.resources", None, &newer_sample));
        assert!(!live.apply("host.resources", None, &sample()));
        // The item started; a stale queued copy cannot roll it back.
        let mut running = item("a", "running", 0, 10);
        running["sessionId"] = json!("s-9");
        assert!(live.apply("queue.changed", None, &json!({"item": running})));
        assert!(!live.apply(
            "queue.changed",
            None,
            &json!({"item": item("a", "queued", 0, 10)})
        ));
        // A forwarded peer event lands on the peer.
        assert!(live.apply(
            "queue.changed",
            Some("gpu-box"),
            &json!({"item": item("p9", "queued", 1, 50)})
        ));
        assert!(live.apply(
            "queue.changed",
            None,
            &json!({"hub": "gpu-box", "item": item("p8", "queued", 0, 40)})
        ));
        assert!(!live.apply("queue.changed", None, &json!({"item": {}})));
        assert!(!live.apply("agent.snapshot", None, &json!({})));

        let cards = cards(&snapshot, &live);
        assert_eq!(
            cards[0].resources.as_ref().unwrap().gate_label(),
            "Gate open"
        );
        assert_eq!(cards[0].items[0].state, State::Running);
        assert_eq!(cards[0].items[0].note(), "Session s-9");
        let peer: Vec<_> = cards[1].items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(peer, ["p9", "p8"], "priority first");

        // A snapshot that shows the item running settles the overlay.
        let mut fresh = snapshot_value();
        fresh["list"]["items"] = json!([{"id": "a", "state": "running", "enqueuedAt": 10}]);
        fresh["resources"] = newer_sample;
        live.settle(&Snapshot::parse(&fresh));
        assert!(live.items.get(LOCAL).unwrap().is_empty());
        assert!(!live.resources.contains_key(LOCAL));
        assert_eq!(live.items.get("gpu-box").unwrap().len(), 2);
    }

    #[test]
    fn spawn_submissions() {
        let submit = SpawnSubmit {
            host: HostChoice::Peer("gpu-box".into()),
            provider: "claude".into(),
            cwd: "/srv/api".into(),
            prompt: " Fix the flaky test ".into(),
            model: "opus".into(),
            effort: String::new(),
            permission_mode: "default".into(),
            label: "flaky".into(),
        };
        assert_eq!(
            submit.params().unwrap(),
            json!({"host": "gpu-box", "kind": "spawn", "label": "flaky",
                "source": {"kind": "prompt"},
                "spawn": {"provider": "claude", "cwd": "/srv/api", "prompt": "Fix the flaky test",
                          "model": "opus", "permissionMode": "default"}})
        );
        let best = SpawnSubmit {
            host: HostChoice::Best,
            label: String::new(),
            ..submit.clone()
        };
        assert_eq!(best.params().unwrap()["host"], "any");
        assert!(best.params().unwrap().get("label").is_none());
        assert!(
            SpawnSubmit {
                prompt: " ".into(),
                ..submit.clone()
            }
            .params()
            .is_err()
        );
        assert!(
            SpawnSubmit {
                cwd: "relative".into(),
                ..submit.clone()
            }
            .params()
            .is_err()
        );
        assert!(
            SpawnSubmit {
                provider: "x".into(),
                ..submit
            }
            .params()
            .is_err()
        );

        let (host, item) = parse_submitted(&json!({"host": "gpu-box",
            "item": {"id": "q_1", "state": "queued", "kind": "spawn"}}))
        .unwrap();
        assert_eq!(submitted_line(&host, &item), "Queued on gpu-box as q_1.");
        let (host, item) =
            parse_submitted(&json!({"item": {"id": "q_2", "state": "running"}})).unwrap();
        assert_eq!(
            submitted_line(&host, &item),
            "Started on this machine as q_2."
        );
        assert!(parse_submitted(&json!({"host": "x"})).is_none());
    }
}
