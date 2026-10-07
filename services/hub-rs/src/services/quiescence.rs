//! Server-owned continuous fleet-rest evidence. Missing or stale evidence is a
//! blocker; caller parameters never supply state or tune the dwell.
pub mod power;
pub mod sampler;
pub mod source;
use anyhow::{Context, Result, bail};
pub use sampler::{Watcher, install};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Mutex};
#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub connection_id: u64,
    pub label: String,
    pub activity_seq: u64,
    /// Broker chooses last interaction for input-reporting clients, otherwise
    /// last call/publication/connect time for legacy clients.
    pub idle_active_ms: i64,
    pub provider: bool,
    pub plugin: bool,
    pub internal: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: String,
    pub name: String,
    pub action_kind: String,
    pub next_run_ms: Option<i64>,
    pub running: bool,
}
#[derive(Clone, Debug)]
pub struct PeerSessions {
    pub name: String,
    pub connected: bool,
    pub sessions: std::result::Result<Value, String>,
}
#[derive(Clone, Debug)]
pub struct Evidence {
    pub now_ms: i64,
    pub sessions: std::result::Result<Value, String>,
    pub clients: std::result::Result<Vec<ClientInfo>, String>,
    pub jobs: std::result::Result<Vec<JobInfo>, String>,
    pub peers: std::result::Result<Vec<PeerSessions>, String>,
    /// Additional host-owned admission/workflow operations not yet represented
    /// by a daemon session. Never populated from the query payload.
    pub operations: Vec<Blocker>,
}
impl Evidence {
    pub fn unknown(now_ms: i64) -> Self {
        Self {
            now_ms,
            sessions: Err("session evidence unavailable".into()),
            clients: Err("broker client evidence unavailable".into()),
            jobs: Err("job evidence unavailable".into()),
            peers: Err("peer evidence unavailable".into()),
            operations: vec![],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Tunables {
    pub dwell_ms: i64,
    pub client_idle_ms: i64,
    pub job_lookahead_ms: i64,
    pub max_sample_gap_ms: i64,
    pub keep_jobs_awake: bool,
}
impl Default for Tunables {
    fn default() -> Self {
        Self {
            dwell_ms: 12 * 60_000,
            client_idle_ms: 10 * 60_000,
            job_lookahead_ms: 15 * 60_000,
            max_sample_gap_ms: 3 * 60_000,
            keep_jobs_awake: false,
        }
    }
}
impl Tunables {
    fn normalized(mut self) -> Self {
        let d = Self::default();
        if self.dwell_ms <= 0 {
            self.dwell_ms = d.dwell_ms;
        }
        if self.client_idle_ms <= 0 {
            self.client_idle_ms = d.client_idle_ms;
        }
        if self.job_lookahead_ms <= 0 {
            self.job_lookahead_ms = d.job_lookahead_ms;
        }
        if self.max_sample_gap_ms <= 0 {
            self.max_sample_gap_ms = d.max_sample_gap_ms;
        }
        self
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Blocker {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub detail: String,
}
impl Blocker {
    pub fn new(kind: &str, id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            detail: detail.into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub quiescent: bool,
    pub since: Option<i64>,
    pub blockers: Vec<Blocker>,
    pub dwell_seconds: i64,
    pub calm_seconds: i64,
}
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub peer: String,
    pub mode: String,
    pub ambient: String,
    pub background_tasks: i64,
    pub pending_approval: bool,
    pub pending_question: bool,
    pub ended: bool,
    pub unreadable: Option<String>,
}
fn first_string<'a>(row: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|key| row[*key].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("")
}
pub fn parse_sessions(peer: &str, raw: &Value) -> Result<Vec<Session>> {
    let rows = raw
        .as_array()
        .context("null/malformed session list is not an empty fleet")?;
    let mut sessions = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if !row.is_null() && !row.is_object() {
            bail!("unreadable session list: row {index} is not an object");
        }
        let id = first_string(row, &["sessionId", "session_id"]);
        let mode = first_string(row, &["mode"]);
        let ambient = first_string(row, &["ambientState", "state"]);
        let ended = first_string(row, &["status"]) == "ended" || mode == "stopped";
        let background = ["backgroundTasks", "background_tasks"]
            .iter()
            .find_map(|key| row[*key].as_f64().filter(|v| *v > 0.).map(|v| v as i64))
            .unwrap_or(0);
        sessions.push(Session {
            id: if id.is_empty() {
                format!("row-{index}")
            } else {
                id.into()
            },
            peer: peer.into(),
            mode: mode.into(),
            ambient: ambient.into(),
            background_tasks: background,
            pending_approval: row.get("pendingApproval").is_some_and(|v| !v.is_null())
                || row["pending"]["kind"] == "approval",
            pending_question: row["pendingQuestions"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
                || row["pending"]["kind"] == "question",
            ended: !id.is_empty() && ended,
            unreadable: if id.is_empty() {
                Some("no session id".into())
            } else if mode.is_empty() && ambient.is_empty() && !ended {
                Some("no mode and no ambientState on row".into())
            } else {
                None
            },
        });
    }
    Ok(sessions)
}
fn session_blockers(sessions: &[Session]) -> Vec<Blocker> {
    let mut blockers = Vec::new();
    for s in sessions {
        if s.ended {
            continue;
        }
        let id = if s.peer.is_empty() {
            s.id.clone()
        } else {
            format!("hub:{}/{}", s.peer, s.id)
        };
        if let Some(error) = &s.unreadable {
            blockers.push(Blocker::new(
                "session-unreadable",
                id,
                format!(
                    "Session row could not be read ({error}); finished state cannot be established"
                ),
            ));
            continue;
        }
        let (state, kind) = if !s.mode.is_empty() {
            (
                s.mode.as_str(),
                match s.mode.as_str() {
                    "responding" => Some("session-working"),
                    "input" => None,
                    "approval" => Some("pending-approval"),
                    "question" => Some("pending-question"),
                    _ => Some("session-unknown"),
                },
            )
        } else {
            (
                s.ambient.as_str(),
                match s.ambient.as_str() {
                    "thinking" | "streaming" | "background" => Some("session-working"),
                    "idle" => None,
                    "waiting_approval" => Some("pending-approval"),
                    "waiting_input" => Some("pending-question"),
                    _ => Some("session-unknown"),
                },
            )
        };
        if let Some(kind) = kind {
            blockers.push(Blocker::new(kind,id.clone(),if kind=="session-unknown"{format!("State {state}: SPAWNING/resuming or a TERMINAL. Terminal command activity is untracked; a live terminal blocks until closed")}else{format!("Session state {state}")}));
        }
        if s.background_tasks > 0 {
            blockers.push(Blocker::new(
                "background-tasks",
                id.clone(),
                format!(
                    "{} background tasks remain: dev server, watcher or agent poll loop",
                    s.background_tasks
                ),
            ));
        }
        if s.pending_approval {
            blockers.push(Blocker::new(
                "pending-approval",
                id.clone(),
                "Waiting on a permission decision; a sleeping machine sends no push",
            ));
        }
        if s.pending_question {
            blockers.push(Blocker::new(
                "pending-question",
                id,
                "Waiting on an answer to a question",
            ));
        }
    }
    blockers
}
pub fn evaluate(input: &Evidence, tun: Tunables, asked: &BTreeMap<u64, u64>) -> Vec<Blocker> {
    let tun = tun.normalized();
    let mut out = input.operations.clone();
    match &input.sessions {
        Ok(raw) => match parse_sessions("", raw) {
            Ok(rows) => out.extend(session_blockers(&rows)),
            Err(e) => out.push(Blocker::new(
                "fleet-unreadable",
                "",
                format!("Could not read fleet: {e}; no answer is not an empty fleet"),
            )),
        },
        Err(e) => out.push(Blocker::new(
            "fleet-unreadable",
            "",
            format!("Could not read fleet: {e}; no answer is not an empty fleet"),
        )),
    }
    match &input.clients {
        Ok(clients) => {
            for c in clients {
                if c.provider
                    || c.plugin
                    || c.internal
                    || asked.get(&c.connection_id) == Some(&c.activity_seq)
                {
                    continue;
                }
                let idle = input.now_ms.saturating_sub(c.idle_active_ms);
                if idle < tun.client_idle_ms {
                    out.push(Blocker::new(
                        "client-active",
                        "",
                        format!(
                            "{} last acted {} seconds ago; requires {} seconds of silence",
                            c.label,
                            idle.max(0) / 1000,
                            tun.client_idle_ms / 1000
                        ),
                    ));
                }
            }
        }
        Err(e) => out.push(Blocker::new(
            "fleet-unreadable",
            "clients",
            format!("Broker clients could not be read: {e}"),
        )),
    }
    match &input.jobs {
        Ok(jobs) => {
            for j in jobs {
                if j.action_kind == "shell" && !tun.keep_jobs_awake {
                    continue;
                }
                let name = if j.name.is_empty() { &j.id } else { &j.name };
                if j.running {
                    out.push(Blocker::new(
                        "job-running",
                        j.id.clone(),
                        format!("Job {name} is running"),
                    ));
                } else if let Some(next) = j.next_run_ms {
                    if tun.keep_jobs_awake {
                        out.push(Blocker::new(
                            "job-scheduled",
                            j.id.clone(),
                            format!("Job {name} is scheduled; this server must stay awake"),
                        ));
                    } else if next.saturating_sub(input.now_ms) <= tun.job_lookahead_ms {
                        out.push(Blocker::new(
                            "job-due-soon",
                            j.id.clone(),
                            format!(
                                "Job {name} is due in {} seconds",
                                next.saturating_sub(input.now_ms).max(0) / 1000
                            ),
                        ));
                    }
                }
            }
        }
        Err(e) => out.push(Blocker::new(
            "fleet-unreadable",
            "jobs",
            format!("Job schedule could not be read: {e}"),
        )),
    }
    match &input.peers {
        Ok(peers) => {
            for peer in peers {
                if !peer.connected {
                    out.push(Blocker::new(
                        "peer-unreachable",
                        peer.name.clone(),
                        "Federation link is not connected",
                    ));
                    continue;
                }
                match &peer.sessions {
                    Ok(raw) => match parse_sessions(&peer.name, raw) {
                        Ok(rows) => out.extend(session_blockers(&rows)),
                        Err(e) => out.push(Blocker::new(
                            "peer-unreachable",
                            peer.name.clone(),
                            format!("Peer session evidence unreadable: {e}"),
                        )),
                    },
                    Err(e) => out.push(Blocker::new(
                        "peer-unreachable",
                        peer.name.clone(),
                        format!("Peer session evidence unavailable: {e}"),
                    )),
                }
            }
        }
        Err(e) => out.push(Blocker::new(
            "fleet-unreadable",
            "peers",
            format!("Peer configuration/state could not be read: {e}"),
        )),
    }
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.id.cmp(&b.id)));
    out
}
struct MonitorState {
    calm_since: Option<i64>,
    last_sample: Option<i64>,
    blockers: Vec<Blocker>,
}
pub struct Monitor {
    tun: Tunables,
    state: Mutex<MonitorState>,
}
impl Monitor {
    pub fn new(tun: Tunables) -> Self {
        Self {
            tun: tun.normalized(),
            state: Mutex::new(MonitorState {
                calm_since: None,
                last_sample: None,
                blockers: vec![],
            }),
        }
    }
    pub fn observe(&self, input: &Evidence, asked: &BTreeMap<u64, u64>) -> Reading {
        let blockers = evaluate(input, self.tun, asked);
        let mut state = self.state.lock().unwrap();
        if state.last_sample.is_some_and(|last| {
            input.now_ms < last || input.now_ms.saturating_sub(last) > self.tun.max_sample_gap_ms
        }) {
            state.calm_since = None;
        }
        state.last_sample = Some(input.now_ms);
        state.blockers = blockers;
        if !state.blockers.is_empty() {
            state.calm_since = None;
        } else if state.calm_since.is_none() {
            state.calm_since = Some(input.now_ms);
        }
        self.result(&state, input.now_ms)
    }
    pub fn latest(&self, now_ms: i64) -> Reading {
        let state = self.state.lock().unwrap();
        if state.last_sample.is_none_or(|last| {
            now_ms < last || now_ms.saturating_sub(last) > self.tun.max_sample_gap_ms
        }) {
            return Reading {
                quiescent: false,
                since: None,
                dwell_seconds: self.tun.dwell_ms / 1000,
                calm_seconds: 0,
                blockers: vec![Blocker::new(
                    "stale-sample",
                    "",
                    "No current continuous sample; the sampler has not observed this interval",
                )],
            };
        }
        self.result(&state, now_ms)
    }
    fn result(&self, state: &MonitorState, now_ms: i64) -> Reading {
        let mut out = Reading {
            quiescent: false,
            since: None,
            blockers: state.blockers.clone(),
            dwell_seconds: self.tun.dwell_ms / 1000,
            calm_seconds: 0,
        };
        if !out.blockers.is_empty() {
            return out;
        }
        let Some(since) = state.calm_since else {
            out.blockers
                .push(Blocker::new("stale-sample", "", "No calm observation"));
            return out;
        };
        let calm = now_ms.saturating_sub(since).max(0);
        out.calm_seconds = calm / 1000;
        if calm < self.tun.dwell_ms {
            out.blockers.push(Blocker::new(
                "dwell",
                "",
                format!(
                    "No current blockers, but calm held {} of {} seconds",
                    calm / 1000,
                    self.tun.dwell_ms / 1000
                ),
            ));
        } else {
            out.quiescent = true;
            out.since = Some(since);
        }
        out
    }
}

/// Background-safe reads are a fixed host vocabulary, never a caller flag.
// Go's request structs accept case-insensitive field names and reject malformed
// known fields. Ambiguous aliases are conservatively activity: Value no longer
// retains their original wire order.
fn activity_field<'a>(value: &'a Value, name: &str) -> Result<Option<&'a Value>, ()> {
    if value.is_null() {
        return Ok(None);
    }
    let object = value.as_object().ok_or(())?;
    let mut fields = object.iter().filter(|(key, _)| {
        key.chars()
            .map(|c| match c {
                '\u{212a}' => 'k', // encoding/json's Unicode SimpleFold aliases
                '\u{017f}' => 's',
                c => c.to_ascii_lowercase(),
            })
            .eq(name.chars())
    });
    let field = fields.next().map(|(_, value)| value);
    if fields.next().is_some() {
        return Err(());
    }
    Ok(field)
}
fn activity_string(value: Option<&Value>) -> Result<&str, ()> {
    match value {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(value)) => Ok(value),
        _ => Err(()),
    }
}
pub fn passive_call(method: &str, params: &Value) -> bool {
    passive_request(method, Some(params))
}
pub(crate) fn passive_request(method: &str, params: Option<&Value>) -> bool {
    let method = if method.starts_with("hub:") {
        method.split_once('/').map(|(_, m)| m).unwrap_or(method)
    } else {
        method
    };
    let parameter_read = || -> Result<bool, ()> {
        let params = params.ok_or(())?;
        match method {
            "desktop.managerReplacement" => {
                let request = activity_field(params, "request")?.unwrap_or(&Value::Null);
                Ok(activity_string(activity_field(request, "action")?)? == "list")
            }
            "desktop.providerReadiness" => match activity_field(params, "check")? {
                None | Some(Value::Null) => Ok(true),
                Some(Value::Bool(check)) => Ok(!check),
                _ => Err(()),
            },
            _ => {
                let op = activity_string(activity_field(params, "op")?)?;
                let request = activity_field(params, "request")?;
                let nested =
                    activity_string(activity_field(request.unwrap_or(&Value::Null), "op")?)?;
                let op = if method == "desktop.fleetWorkflowRequest" {
                    nested
                } else {
                    op
                };
                Ok(matches!(
                    op,
                    "list"
                        | "get"
                        | "validate"
                        | "requestInbox"
                        | "requestContent"
                        | "next"
                        | "taskReferences"
                ))
            }
        }
    };
    if matches!(
        method,
        "desktop.managerReplacement"
            | "desktop.providerReadiness"
            | "desktop.fleetWorkflowRequest"
            | "fleetWorkflows.request"
    ) {
        return parameter_read().unwrap_or(false);
    }
    matches!(
        method,
        "ui.fonts"
            | "ui.asset"
            | "desktop.worktreeInfo"
            | "desktop.pricingGetRates"
            | "desktop.claudeProfilesAccounts"
            | "desktop.claudeProfilesLoginStatus"
            | "desktop.toolsStatus"
            | "desktop.fleetReviewRead"
            | "desktop.dispatchHistoryRead"
            | "desktop.htmlCardReadDiff"
            | "desktop.loadBriefBoard"
            | "desktop.agentRuntimeStatus"
            | "desktop.keepWarmHeartbeats"
            | "desktop.workflowAgentTranscript"
            | "desktop.workflowAgentConversation"
            | "remote.sharingInfo"
            | "remote.tailscaleInfo"
            | "remote.pairingInfo"
            | "remote.tokensList"
            | "machine.power"
            | "fleet.quiescence"
            | "sessions.snapshots"
            | "sessions.snapshot"
            | "sessions.list"
            | "sessions.recent"
            | "sessions.conversation"
            | "sessions.taskOutput"
            | "sessions.get"
            | "sessions.stats"
            | "sessions.analytics"
            | "agents.list"
            | "agents.get"
            | "config.get"
            | "config.getPath"
            | "layout.get"
            | "layouts.list"
            | "federation.peers"
            | "nodes.list"
            | "brain.info"
            | "app.getCwd"
            | "usage.report"
            | "usage.pacingSchedule"
            | "analytics.summary"
            | "analytics.recent"
            | "providers.checkAll"
            | "providers.listModels"
            | "sessions.load"
            | "git.commitDiff"
            | "claude.listModels"
            | "claude.profiles.list"
            | "claude.sessionsForDir"
            | "plugins.list"
            | "plugins.tools"
            | "plugins.settings"
            | "plugins.manifests"
            | "jobs.list"
            | "jobs.history"
            | "library.list"
            | "push.key"
            | "git.status"
            | "git.diff"
            | "git.numstat"
            | "git.branch"
            | "git.branches"
            | "fs.listDir"
            | "fs.listEntries"
            | "fs.read"
            | "fs.readImage"
            | "fs.readFile"
            | "fs.stat"
            | "fs.watch"
            | "fs.unwatch"
            | "search.project"
            | "git.log"
            | "git.commitNumstat"
            | "app.supervisorHome"
            | "fleet.tasks"
            | "fleet.workers"
            | "fleet.managers"
            | "fleet.dispatches"
            | "routing.preferences.get"
            | "routing.preview"
    )
}
