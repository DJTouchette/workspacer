//! Codex app-server extras beside the chat loop in `codex.rs`: background
//! terminals, plan (collaboration) mode, review, thread goals, thread naming
//! and Windows sandbox readiness.
//!
//! Kept out of `codex.rs` on purpose: the chat driver's turn/approval state
//! machine does not depend on any of this, and none of it may stall it. The
//! driver hands every ws message to [`Extras::intercept`] (our own responses,
//! consumed) and [`Extras::observe`] (everything else, read-only for the chat
//! path), and routes the session's [`CodexControl`] / task-stop channels here.
//!
//! Every request this module sends uses a STRING id (`wks-extras-<n>`), so it
//! can never collide with the driver's numeric turn ids, and every one is
//! optional: an error — a build without the method (real Codex answers
//! `-32600 "Invalid request: unknown variant …"`; `-32601` is the JSON-RPC
//! spelling), or "requires experimentalApi capability" without the opt-in —
//! only marks that feature
//! unavailable (or answers the caller with the reason). Nothing here waits on
//! a reply inside the driver loop.
//!
//! Experimental pieces (`thread/backgroundTerminals/*`,
//! `collaborationMode/list`, `turn/start.collaborationMode`) are used only
//! when the session was spawned with the `codex.experimentalApi` opt-in,
//! which is also what makes the driver send `capabilities.experimentalApi`
//! on `initialize`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use super::AgentUpdate;
use crate::session::background_tasks::{self as bg, BackgroundTask};
use crate::session::codex_controls::{
    CodexControl, CodexGoal, CodexOp, CollaborationShared, WindowsSandboxStatus,
};
use crate::session::store::ManagedTaskStop;
use crate::session::{ConversationStore, SessionStore};

/// Prefix of every request id this module mints.
const ID_PREFIX: &str = "wks-extras-";
/// Bytes of a not-yet-backgrounded terminal's output kept in memory.
const MAX_BUFFERED: usize = 256 * 1024;
/// Bytes one background terminal's log may grow to on disk.
const MAX_LOG: u64 = 8 * 1024 * 1024;
/// Bytes all of one session's terminal logs may grow to together.
const MAX_SESSION_LOGS: u64 = 32 * 1024 * 1024;
/// Longest Codex error text relayed to a caller (Codex's own refusal of an
/// unknown method lists every method it knows — kilobytes).
const MAX_ERROR_TEXT: usize = 300;
/// Task type the background-task list shows for a Codex terminal — the same
/// kind as a Claude background shell, so clients render it the same way.
const TERMINAL_TASK_TYPE: &str = "local_bash";

/// Pure additions to `codex::translate`: review-mode items and Codex's
/// proposed plan (`plan` items, plan collaboration mode).
pub(super) fn translate(method: &str, params: &Value) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    let Some(item) = params.get("item") else {
        return out;
    };
    let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    match (method, ty) {
        ("item/started", "enteredReviewMode") => {
            let what = text("review").unwrap_or("the current changes");
            out.push(AgentUpdate::Notice(format!("Codex review started: {what}")));
        }
        // `exitedReviewMode` arrives as started+completed; the completed one
        // carries the final review — Codex's overall explanation plus one
        // bullet per finding, already rendered as plain text.
        ("item/completed", "exitedReviewMode") => {
            out.push(AgentUpdate::Notice("Codex review finished".into()));
            if let Some(review) = text("review") {
                out.push(AgentUpdate::AssistantText(review.to_string()));
            }
        }
        // The completed plan item is authoritative (its deltas may not
        // concatenate to it), so only it is rendered.
        ("item/completed", "plan") => {
            if let Some(plan) = text("text") {
                out.push(AgentUpdate::AssistantText(format!(
                    "\n\n**Proposed plan**\n\n{plan}"
                )));
            }
        }
        _ => {}
    }
    out
}

/// Stamp `collaborationMode` on a `turn/start` when plan mode is (or was)
/// selected. Never sends anything for a session that never chose plan mode,
/// so a session without the opt-in keeps Codex's own defaults untouched.
pub(super) fn stamp_turn(params: &mut Value, collaboration: Option<&CollaborationShared>) {
    let Some(c) = collaboration.filter(|c| c.is_available()) else {
        return;
    };
    let plan = c.is_plan();
    if !plan && !c.engaged.load(Ordering::Relaxed) {
        return;
    }
    // `settings.model` is required by Codex; without it the turn would be
    // refused, so the mode is skipped (and logged) instead.
    let Some(model) = c.model.lock().unwrap().clone() else {
        tracing::warn!(
            "codex plan mode: no model known for the thread; turn sent without collaborationMode"
        );
        return;
    };
    let current = c.effort.lock().unwrap().clone().map(Value::String);
    let effort = if plan {
        c.plan_effort.lock().unwrap().clone().or(current)
    } else {
        current
    };
    params["collaborationMode"] = json!({
        "mode": if plan { "plan" } else { "default" },
        "settings": {
            "model": model,
            "reasoning_effort": effort.unwrap_or(Value::Null),
            // null = Codex's built-in instructions for the selected mode.
            "developer_instructions": Value::Null,
        }
    });
    if plan {
        c.engaged.store(true, Ordering::Relaxed);
    }
}

/// What one of our in-flight requests was for.
enum Inflight {
    Control {
        method: &'static str,
        op: CodexOp,
        reply: oneshot::Sender<Result<Value, String>>,
    },
    TaskStop {
        task_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    GoalGet,
    CollaborationList,
    Readiness,
    TerminalsList,
}

/// A unified-exec process Codex started for this thread. It becomes a row
/// in the session's background-task list once it is proven to be
/// backgrounded: still running when its turn ends, written to with
/// `write_stdin` (`terminalInteraction`), or listed by
/// `thread/backgroundTerminals/list`.
struct Terminal {
    item_id: String,
    process_id: String,
    command: String,
    task_id: String,
    promoted: bool,
    /// Output seen before promotion, flushed into the log when promoted.
    buffered: String,
    log: Option<PathBuf>,
    logged: u64,
}

pub(super) struct Extras {
    session_id: String,
    cwd: String,
    experimental: bool,
    windows: bool,
    next_id: u64,
    inflight: HashMap<String, Inflight>,
    collaboration: Arc<CollaborationShared>,
    /// Terminals by Codex item id.
    terminals: HashMap<String, Terminal>,
    goal_requested: bool,
    /// This session's own log directory (see `codex_terminal_logs`); `None`
    /// when the platform names no local data dir (logs are then not kept).
    log_root: Option<PathBuf>,
    /// Bytes written to this session's logs, against [`MAX_SESSION_LOGS`].
    logged_total: u64,
    /// The live ask/yolo flag: a bypassed session does not run in Codex's
    /// Windows sandbox, so its readiness is not news.
    yolo: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// The Windows sandbox notice was posted (at most once per session).
    sandbox_noticed: bool,
    /// The bootstrap `thread/start` (id 101) result was taken.
    started_seen: bool,
    /// The world-writable warning was posted (at most once per session).
    writable_noticed: bool,
}

impl Drop for Extras {
    /// The driver is ending: its background terminals died with its
    /// app-server, and their logs go with them.
    fn drop(&mut self) {
        if let Some(dir) = &self.log_root {
            super::codex_terminal_logs::remove_session(dir);
        }
    }
}

/// The JSON-RPC error's text, worded for a person.
fn rpc_error(method: &str, err: &Value) -> String {
    let code = err.get("code").and_then(Value::as_i64);
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    let lower = message.to_ascii_lowercase();
    // Real Codex answers a method it does not know with -32600 "Invalid
    // request: unknown variant `X`, expected one of …" (its whole method
    // list); -32601 / "method not found" is the JSON-RPC spelling.
    if code == Some(-32601)
        || lower.contains("method not found")
        || lower.contains("unknown variant")
    {
        format!("this Codex build does not support {method}")
    } else if message.contains("experimentalApi") {
        format!("{method} needs the codex.experimentalApi opt-in")
    } else {
        let mut end = message.len().min(MAX_ERROR_TEXT);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        let ellipsis = if end < message.len() { "…" } else { "" };
        format!("{method} failed: {}{ellipsis}", &message[..end])
    }
}

fn now_ms() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000
}

/// A background-task id for a Codex terminal: `codex-<processId>` when that
/// is a plain id, else derived from the item id.
fn terminal_task_id(process_id: &str, item_id: &str) -> String {
    let candidate = format!("codex-{process_id}");
    if bg::valid_task_id(&candidate) {
        return candidate;
    }
    let clean: String = item_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(50)
        .collect();
    format!(
        "codex-{}",
        if clean.is_empty() { "terminal" } else { &clean }
    )
}

impl Extras {
    pub(super) fn new(
        session_id: &str,
        cwd: &str,
        experimental: bool,
        collaboration: Arc<CollaborationShared>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            cwd: cwd.to_string(),
            experimental,
            windows: cfg!(windows),
            next_id: 0,
            inflight: HashMap::new(),
            collaboration,
            terminals: HashMap::new(),
            goal_requested: false,
            log_root: super::codex_terminal_logs::root().map(|root| root.join(session_id)),
            logged_total: 0,
            yolo: None,
            sandbox_noticed: false,
            started_seen: false,
            writable_noticed: false,
        }
    }

    /// Share the session's live ask/yolo flag (registered after `new`).
    pub(super) fn watch_yolo(&mut self, yolo: Arc<std::sync::atomic::AtomicBool>) {
        self.yolo = Some(yolo);
    }

    fn bypassed(&self) -> bool {
        self.yolo
            .as_ref()
            .is_some_and(|y| y.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    pub(super) fn with_platform(mut self, windows: bool, log_root: PathBuf) -> Self {
        self.windows = windows;
        self.log_root = Some(log_root);
        self
    }

    /// `initialize.params.capabilities`, when the opt-in asks for it.
    pub(super) fn initialize_capabilities(&self) -> Option<Value> {
        self.experimental
            .then(|| json!({ "experimentalApi": true }))
    }

    fn send(
        &mut self,
        out_tx: &mpsc::UnboundedSender<Value>,
        method: &str,
        params: Value,
        purpose: Inflight,
    ) {
        self.next_id += 1;
        let id = format!("{ID_PREFIX}{}", self.next_id);
        self.inflight.insert(id.clone(), purpose);
        let mut request = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            request["params"] = params;
        }
        let _ = out_tx.send(request);
    }

    /// Requests that need no thread, sent right behind `initialize`.
    pub(super) fn startup(&mut self, out_tx: &mpsc::UnboundedSender<Value>) {
        if self.windows {
            self.send(
                out_tx,
                "windowsSandbox/readiness",
                Value::Null,
                Inflight::Readiness,
            );
        }
        // Without the opt-in plan mode stays unavailable and the snapshot
        // says nothing, rather than advertise a toggle that cannot work.
        if self.experimental {
            self.send(
                out_tx,
                "collaborationMode/list",
                json!({}),
                Inflight::CollaborationList,
            );
        }
    }

    fn ours(params: &Value, thread_id: Option<&str>) -> bool {
        match (params.get("threadId").and_then(Value::as_str), thread_id) {
            (Some(theirs), Some(ours)) => theirs == ours,
            _ => true,
        }
    }

    fn notice(&self, conv: &ConversationStore, text: String) {
        if let Some(item) = super::conversation_item(&AgentUpdate::Notice(text)) {
            conv.push(&self.session_id, vec![item]);
        }
    }

    /// Consume a response to one of OUR requests. Returns false for
    /// everything else, which the chat driver handles as before.
    pub(super) fn intercept(
        &mut self,
        value: &Value,
        store: &SessionStore,
        conv: &ConversationStore,
        out_tx: &mpsc::UnboundedSender<Value>,
        thread_id: Option<&str>,
    ) -> bool {
        if value.get("method").is_some() {
            return false;
        }
        let Some(id) = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with(ID_PREFIX))
        else {
            return false;
        };
        let Some(purpose) = self.inflight.remove(id) else {
            return true;
        };
        let result = match (value.get("error"), value.get("result")) {
            (Some(err), _) => Err(err.clone()),
            (None, Some(result)) => Ok(result.clone()),
            (None, None) => Ok(Value::Null),
        };
        match purpose {
            Inflight::Control { method, op, reply } => {
                let outcome = match result {
                    Err(err) => Err(rpc_error(method, &err)),
                    Ok(result) => {
                        self.applied(&op, &result, store);
                        Ok(result)
                    }
                };
                let _ = reply.send(outcome);
            }
            Inflight::TaskStop { task_id, reply } => {
                let outcome =
                    match result {
                        Err(err) => Err(rpc_error("thread/backgroundTerminals/terminate", &err)),
                        Ok(result)
                            if result.get("terminated").and_then(Value::as_bool) == Some(false) =>
                        {
                            Err("Codex reports no running terminal by that id".to_string())
                        }
                        Ok(_) => {
                            let now = now_ms();
                            store.update_managed_background_tasks(&self.session_id, |tasks| {
                                match tasks.iter_mut().find(|t| t.id == task_id && t.is_running()) {
                                    Some(task) => {
                                        task.status = "stopped".into();
                                        task.ended_at = Some(now);
                                        true
                                    }
                                    None => false,
                                }
                            });
                            Ok(())
                        }
                    };
                let _ = reply.send(outcome);
            }
            Inflight::GoalGet => match result {
                Ok(result) => {
                    let goal = result.get("goal").and_then(CodexGoal::from_wire);
                    self.set_goal(store, goal);
                }
                Err(err) => tracing::debug!(session = %self.session_id, error = %err,
                    "codex thread/goal/get unavailable — no goal on the snapshot"),
            },
            Inflight::CollaborationList => match result {
                Ok(result) => self.collaboration_listed(&result, store),
                Err(err) => tracing::info!(session = %self.session_id, error = %err,
                    "codex collaborationMode/list unavailable — plan mode off for this session"),
            },
            Inflight::Readiness => match result {
                Ok(result) => self.readiness(&result, store, conv),
                Err(err) => tracing::debug!(session = %self.session_id, error = %err,
                    "codex windowsSandbox/readiness unavailable"),
            },
            Inflight::TerminalsList => match result {
                Ok(result) => self.terminals_listed(&result, store, out_tx, thread_id),
                Err(err) => tracing::debug!(session = %self.session_id, error = %err,
                    "codex thread/backgroundTerminals/list unavailable"),
            },
        }
        true
    }

    /// Reflect a successful control on the snapshot at once, so a client
    /// that issued it sees the change without waiting for the notification.
    fn applied(&mut self, op: &CodexOp, result: &Value, store: &SessionStore) {
        match op {
            CodexOp::SetGoal { .. } => {
                if let Some(goal) = result.get("goal").and_then(CodexGoal::from_wire) {
                    self.set_goal(store, Some(goal));
                }
            }
            CodexOp::ClearGoal => self.set_goal(store, None),
            CodexOp::SetThreadName { name } => self.set_thread_name(store, Some(name.clone())),
            CodexOp::WindowsSandboxSetup { .. } => {
                if result.get("started").and_then(Value::as_bool) != Some(false) {
                    store.update_state(&self.session_id, |s| {
                        s.windows_sandbox = Some(WindowsSandboxStatus {
                            ready: false,
                            status: "settingUp".into(),
                            detail: None,
                        });
                        true
                    });
                }
            }
            CodexOp::Review { .. } => {}
        }
    }

    fn set_goal(&self, store: &SessionStore, goal: Option<CodexGoal>) {
        store.update_state(&self.session_id, |s| {
            if s.goal == goal {
                return false;
            }
            s.goal = goal;
            true
        });
    }

    fn set_thread_name(&self, store: &SessionStore, name: Option<String>) {
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        store.update_state(&self.session_id, |s| {
            if s.thread_name == name {
                return false;
            }
            s.thread_name = name;
            true
        });
    }

    fn collaboration_listed(&mut self, result: &Value, store: &SessionStore) {
        let presets = result
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let Some(plan) = presets
            .iter()
            .find(|p| p.get("mode").and_then(Value::as_str) == Some("plan"))
        else {
            tracing::info!(session = %self.session_id, "codex offers no plan collaboration mode");
            return;
        };
        if let Some(effort) = plan.get("reasoning_effort") {
            *self.collaboration.plan_effort.lock().unwrap() = Some(effort.clone());
        }
        self.collaboration.available.store(true, Ordering::Relaxed);
        let mode = if self.collaboration.is_plan() {
            "plan"
        } else {
            "default"
        };
        store.update_state(&self.session_id, |s| {
            s.collaboration_mode = Some(mode.to_string());
            true
        });
    }

    fn readiness(&mut self, result: &Value, store: &SessionStore, conv: &ConversationStore) {
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let ready = status == "ready";
        let previous = store
            .get(&self.session_id)
            .and_then(|s| s.windows_sandbox)
            .map(|w| w.ready);
        store.update_state(&self.session_id, |s| {
            let detail = s.windows_sandbox.as_ref().and_then(|w| w.detail.clone());
            s.windows_sandbox = Some(WindowsSandboxStatus {
                ready,
                status: status.clone(),
                detail: if ready { None } else { detail },
            });
            true
        });
        // Only a session that really runs in Codex's Windows sandbox (not
        // bypassed) hears about it, and only once.
        if !ready && previous != Some(false) && !self.bypassed() && !self.sandbox_noticed {
            self.sandbox_noticed = true;
            let what = match status.as_str() {
                "notConfigured" => "is not set up",
                "updateRequired" => "needs an update",
                _ => "is not ready",
            };
            self.notice(
                conv,
                format!(
                    "Codex's Windows sandbox {what} ({status}). Sandboxed commands may fail until setup runs — start Windows sandbox setup for this session."
                ),
            );
        } else if ready && previous == Some(false) {
            self.notice(conv, "Codex's Windows sandbox is ready.".into());
        }
    }

    /// Read-only pass over every ws message (after the chat driver).
    pub(super) fn observe(
        &mut self,
        value: &Value,
        store: &SessionStore,
        conv: &ConversationStore,
        out_tx: &mpsc::UnboundedSender<Value>,
        thread_id: Option<&str>,
        subscribed: bool,
    ) {
        // Once we hold the thread, fetch its goal (once per driver life).
        if subscribed && !self.goal_requested {
            if let Some(tid) = thread_id {
                self.goal_requested = true;
                self.send(
                    out_tx,
                    "thread/goal/get",
                    json!({ "threadId": tid }),
                    Inflight::GoalGet,
                );
            }
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            // Our thread/start (101) / thread/resume (2) result carries the
            // thread's name, model and effort.
            // Id 101 is the bootstrap thread/start only once: a user turn's
            // ever-climbing id can reach it later, so it is taken at most
            // once and only when it carries a thread (like codex.rs's
            // `thread_id.is_none()` guard). Turn ids start at 3, so id 2 is
            // always a thread/resume.
            if let Some(result) = value.get("result").filter(|r| r.get("thread").is_some()) {
                match value.get("id").and_then(Value::as_u64) {
                    Some(2) => self.thread_result(result, store),
                    Some(101) if !self.started_seen => {
                        self.started_seen = true;
                        self.thread_result(result, store);
                    }
                    _ => {}
                }
            }
            return;
        };
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        if !Self::ours(&params, thread_id) {
            return;
        }
        match method {
            "thread/name/updated" => {
                let name = params
                    .get("threadName")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.set_thread_name(store, name);
            }
            "thread/goal/updated" => {
                if let Some(goal) = params.get("goal").and_then(CodexGoal::from_wire) {
                    self.set_goal(store, Some(goal));
                }
            }
            "thread/goal/cleared" => self.set_goal(store, None),
            "thread/settings/updated" => {
                let settings = params.get("threadSettings").unwrap_or(&Value::Null);
                if let Some(model) = settings.get("model").and_then(Value::as_str) {
                    *self.collaboration.model.lock().unwrap() = Some(model.to_string());
                }
                if let Some(effort) = settings.get("effort").and_then(Value::as_str) {
                    *self.collaboration.effort.lock().unwrap() = Some(effort.to_string());
                }
                // The thread's own collaboration mode — ours, or one the TUI
                // switched — is the truth the next turn starts from.
                self.remote_mode(settings.get("collaborationMode"), store);
            }
            "windows/worldWritableWarning" => self.world_writable(&params, store, conv),
            "windowsSandbox/setupCompleted" => self.setup_completed(&params, store, conv, out_tx),
            "item/started" => self.item_started(&params, store),
            "item/completed" => self.item_completed(&params, store),
            "item/commandExecution/outputDelta" => self.output_delta(&params),
            "item/commandExecution/terminalInteraction" => {
                let item = params.get("itemId").and_then(Value::as_str).unwrap_or("");
                let process = params.get("processId").and_then(Value::as_str);
                let key = self
                    .terminals
                    .values()
                    .find(|t| t.item_id == item || Some(t.process_id.as_str()) == process)
                    .map(|t| t.item_id.clone());
                if let Some(key) = key {
                    self.promote(&key, store);
                }
            }
            // A process still running when its turn is over IS a background
            // terminal. With the opt-in, Codex's own list reconciles the rest.
            "turn/completed" => {
                let running: Vec<String> = self
                    .terminals
                    .values()
                    .filter(|t| !t.promoted)
                    .map(|t| t.item_id.clone())
                    .collect();
                for key in running {
                    self.promote(&key, store);
                }
                if self.experimental {
                    if let Some(tid) = thread_id {
                        self.send(
                            out_tx,
                            "thread/backgroundTerminals/list",
                            json!({ "threadId": tid }),
                            Inflight::TerminalsList,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// Fold a collaboration mode Codex reports for the thread (a resume
    /// restores the persisted one; `thread/settings/updated` carries it on
    /// every change) into the plan-mode state, so a thread already in plan
    /// mode stays in it — and leaving it is sent explicitly.
    fn remote_mode(&mut self, mode: Option<&Value>, store: &SessionStore) {
        let Some(kind) = mode.and_then(|m| m.get("mode")).and_then(Value::as_str) else {
            return;
        };
        let plan = kind == "plan";
        self.collaboration.set_plan(plan);
        // The thread already holds this mode: plan → later non-plan turns must
        // say "default"; default → nothing needs saying.
        self.collaboration.engaged.store(plan, Ordering::Relaxed);
        if let Some(effort) = mode
            .and_then(|m| m.pointer("/settings/reasoning_effort"))
            .and_then(Value::as_str)
        {
            if plan {
                *self.collaboration.plan_effort.lock().unwrap() = Some(json!(effort));
            }
        }
        // Without the opt-in the toggle is unavailable: only a thread that
        // really is in plan mode (switched in the TUI) is worth showing.
        let label = match (plan, self.collaboration.is_available()) {
            (true, _) => Some("plan"),
            (false, true) => Some("default"),
            (false, false) => None,
        };
        store.update_state(&self.session_id, |s| {
            if s.collaboration_mode.as_deref() == label {
                return false;
            }
            s.collaboration_mode = label.map(str::to_owned);
            true
        });
    }

    fn thread_result(&mut self, result: &Value, store: &SessionStore) {
        let mode = result
            .get("collaborationMode")
            .or_else(|| result.pointer("/thread/collaborationMode"))
            .or_else(|| result.pointer("/threadSettings/collaborationMode"))
            .cloned();
        self.remote_mode(mode.as_ref(), store);
        if let Some(model) = result.get("model").and_then(Value::as_str) {
            *self.collaboration.model.lock().unwrap() = Some(model.to_string());
        }
        if let Some(effort) = result.get("reasoningEffort").and_then(Value::as_str) {
            *self.collaboration.effort.lock().unwrap() = Some(effort.to_string());
        }
        if let Some(name) = result
            .get("thread")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
        {
            self.set_thread_name(store, Some(name.to_string()));
        }
    }

    fn world_writable(&mut self, params: &Value, store: &SessionStore, conv: &ConversationStore) {
        let paths: Vec<&str> = params
            .get("samplePaths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(5)
            .collect();
        let extra = params
            .get("extraCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let detail = if params.get("failedScan").and_then(Value::as_bool) == Some(true) {
            "Codex could not finish scanning for world-writable folders".to_string()
        } else {
            let mut d = format!(
                "world-writable folders the sandbox cannot protect: {}",
                paths.join(", ")
            );
            if extra > 0 {
                d.push_str(&format!(" (+{extra} more)"));
            }
            d
        };
        store.update_state(&self.session_id, |s| {
            let mut status = s.windows_sandbox.clone().unwrap_or(WindowsSandboxStatus {
                ready: true,
                status: "ready".into(),
                detail: None,
            });
            status.detail = Some(detail.clone());
            s.windows_sandbox = Some(status);
            true
        });
        if !self.bypassed() && !self.writable_noticed {
            self.writable_noticed = true;
            self.notice(conv, format!("Windows sandbox warning: {detail}."));
        }
    }

    fn setup_completed(
        &mut self,
        params: &Value,
        store: &SessionStore,
        conv: &ConversationStore,
        out_tx: &mpsc::UnboundedSender<Value>,
    ) {
        if params.get("success").and_then(Value::as_bool) == Some(true) {
            // Codex's word on readiness, not ours.
            self.send(
                out_tx,
                "windowsSandbox/readiness",
                Value::Null,
                Inflight::Readiness,
            );
            return;
        }
        let error = params
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("setup did not complete")
            .to_string();
        store.update_state(&self.session_id, |s| {
            s.windows_sandbox = Some(WindowsSandboxStatus {
                ready: false,
                status: "setupFailed".into(),
                detail: Some(error.clone()),
            });
            true
        });
        self.notice(
            conv,
            format!("Codex's Windows sandbox setup failed: {error}"),
        );
    }

    // ── background terminals ────────────────────────────────────────────

    fn item_started(&mut self, params: &Value, store: &SessionStore) {
        let Some(item) = params.get("item") else {
            return;
        };
        if item.get("type").and_then(Value::as_str) != Some("commandExecution") {
            return;
        }
        let source = item.get("source").and_then(Value::as_str).unwrap_or("");
        let (Some(id), Some(process)) = (
            item.get("id").and_then(Value::as_str),
            item.get("processId").and_then(Value::as_str),
        ) else {
            return;
        };
        // A `write_stdin` poll is its own item naming an existing process:
        // that process is being driven in the background.
        if source == "unifiedExecInteraction" {
            let key = self
                .terminals
                .values()
                .find(|t| t.process_id == process)
                .map(|t| t.item_id.clone());
            if let Some(key) = key {
                self.promote(&key, store);
            }
            return;
        }
        if !source.is_empty() && source != "unifiedExecStartup" {
            return;
        }
        self.terminals.insert(
            id.to_string(),
            Terminal {
                item_id: id.to_string(),
                process_id: process.to_string(),
                command: item
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                task_id: terminal_task_id(process, id),
                promoted: false,
                buffered: String::new(),
                log: None,
                logged: 0,
            },
        );
    }

    fn output_delta(&mut self, params: &Value) {
        let (Some(item), Some(delta)) = (
            params.get("itemId").and_then(Value::as_str),
            params.get("delta").and_then(Value::as_str),
        ) else {
            return;
        };
        let Some(t) = self.terminals.get_mut(item) else {
            return;
        };
        if t.promoted {
            append_log(t, delta, &mut self.logged_total);
        } else if t.buffered.len() < MAX_BUFFERED {
            let room = MAX_BUFFERED - t.buffered.len();
            let mut end = delta.len().min(room);
            while !delta.is_char_boundary(end) {
                end -= 1;
            }
            t.buffered.push_str(&delta[..end]);
        }
    }

    fn promote(&mut self, item_id: &str, store: &SessionStore) {
        let Some(t) = self.terminals.get_mut(item_id) else {
            return;
        };
        if t.promoted {
            return;
        }
        t.promoted = true;
        if let Some(root) = &self.log_root {
            let log = root.join(format!("{}.output", t.task_id));
            let room = MAX_SESSION_LOGS.saturating_sub(self.logged_total);
            let mut keep = t.buffered.len().min(room as usize);
            while !t.buffered.is_char_boundary(keep) {
                keep -= 1;
            }
            let opened = super::codex_terminal_logs::ensure_private_dir(root).and_then(|_| {
                // A finished terminal whose process id was reused left its
                // (our own) log behind; a new one starts fresh.
                let _ = std::fs::remove_file(&log);
                super::codex_terminal_logs::create(&log, &t.buffered.as_bytes()[..keep])
            });
            match opened {
                Ok(()) => {
                    t.logged = keep as u64;
                    self.logged_total += keep as u64;
                    t.log = Some(log);
                }
                Err(error) => {
                    tracing::warn!(session = %self.session_id, path = %log.display(), %error,
                    "codex background terminal: cannot write its log")
                }
            }
        }
        t.buffered = String::new();
        let (task_id, command, has_log) = (t.task_id.clone(), t.command.clone(), t.log.clone());
        let now = now_ms();
        store.update_managed_background_tasks(&self.session_id, |tasks| {
            if let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) {
                if task.is_running() {
                    return false;
                }
                // The same process id reused by a later terminal.
                tasks.retain(|task| task.id != task_id);
            }
            let mut task = BackgroundTask::new(&task_id, TERMINAL_TASK_TYPE, now);
            task.description = (!command.is_empty()).then(|| command.clone());
            task.tool_use_id = Some(item_id.to_string());
            task.has_output = has_log.is_some();
            task.output_file = has_log;
            tasks.push(task);
            let before: Vec<(String, Option<PathBuf>)> = tasks
                .iter()
                .map(|t| (t.id.clone(), t.output_file.clone()))
                .collect();
            bg::retain_bounded(tasks, now);
            // An evicted row's log goes with it.
            for (id, file) in before {
                if let Some(file) = file.filter(|_| !tasks.iter().any(|t| t.id == id)) {
                    let _ = std::fs::remove_file(file);
                }
            }
            true
        });
    }

    fn item_completed(&mut self, params: &Value, store: &SessionStore) {
        let Some(item) = params.get("item") else {
            return;
        };
        if item.get("type").and_then(Value::as_str) != Some("commandExecution") {
            return;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            return;
        };
        let Some(t) = self.terminals.remove(id) else {
            return;
        };
        if !t.promoted {
            return;
        }
        let exit = item.get("exitCode").and_then(Value::as_i64);
        let status = match item.get("status").and_then(Value::as_str) {
            Some("failed") | Some("declined") => "failed",
            _ if exit.is_some_and(|c| c != 0) => "failed",
            _ => "completed",
        };
        let now = now_ms();
        store.update_managed_background_tasks(&self.session_id, |tasks| {
            let Some(task) = tasks.iter_mut().find(|task| task.id == t.task_id) else {
                return false;
            };
            if !task.is_running() {
                return false;
            }
            task.status = status.into();
            task.ended_at = Some(now);
            task.summary = exit.map(|c| format!("exit code {c}"));
            true
        });
    }

    fn terminals_listed(
        &mut self,
        result: &Value,
        store: &SessionStore,
        out_tx: &mpsc::UnboundedSender<Value>,
        thread_id: Option<&str>,
    ) {
        let listed = result
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for row in &listed {
            let (Some(item), Some(process)) = (
                row.get("itemId").and_then(Value::as_str),
                row.get("processId").and_then(Value::as_str),
            ) else {
                continue;
            };
            if !self.terminals.contains_key(item) {
                // Started before we subscribed: known only from the list.
                self.terminals.insert(
                    item.to_string(),
                    Terminal {
                        item_id: item.to_string(),
                        process_id: process.to_string(),
                        command: row
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        task_id: terminal_task_id(process, item),
                        promoted: false,
                        buffered: String::new(),
                        log: None,
                        logged: 0,
                    },
                );
            }
            self.promote(item, store);
        }
        if let (Some(cursor), Some(tid)) =
            (result.get("nextCursor").and_then(Value::as_str), thread_id)
        {
            self.send(
                out_tx,
                "thread/backgroundTerminals/list",
                json!({ "threadId": tid, "cursor": cursor }),
                Inflight::TerminalsList,
            );
        }
    }

    /// `sessions.taskStop` for a Codex terminal.
    pub(super) fn stop_task(
        &mut self,
        stop: ManagedTaskStop,
        out_tx: &mpsc::UnboundedSender<Value>,
        thread_id: Option<&str>,
    ) {
        let Some(t) = self.terminals.values().find(|t| t.task_id == stop.task_id) else {
            let _ = stop
                .reply
                .send(Err("Codex has no running terminal by that id".into()));
            return;
        };
        if !self.experimental {
            let _ = stop.reply.send(Err(
                "stopping a Codex background terminal needs the codex.experimentalApi opt-in (thread/backgroundTerminals/terminate is experimental)".into(),
            ));
            return;
        }
        let Some(tid) = thread_id else {
            let _ = stop
                .reply
                .send(Err("the Codex thread is not joined yet".into()));
            return;
        };
        let params = json!({ "threadId": tid, "processId": t.process_id });
        self.send(
            out_tx,
            "thread/backgroundTerminals/terminate",
            params,
            Inflight::TaskStop {
                task_id: stop.task_id,
                reply: stop.reply,
            },
        );
    }

    /// One `CodexControl` from the HTTP API.
    pub(super) fn control(
        &mut self,
        control: CodexControl,
        out_tx: &mpsc::UnboundedSender<Value>,
        thread_id: Option<&str>,
    ) {
        let CodexControl { op, reply } = control;
        if let CodexOp::WindowsSandboxSetup { mode } = &op {
            if !self.windows {
                let _ = reply.send(Err(
                    "Windows sandbox setup applies only to Codex running on Windows".into(),
                ));
                return;
            }
            let params = json!({ "mode": mode, "cwd": self.cwd });
            self.send(
                out_tx,
                "windowsSandbox/setupStart",
                params,
                Inflight::Control {
                    method: "windowsSandbox/setupStart",
                    op,
                    reply,
                },
            );
            return;
        }
        let Some(tid) = thread_id else {
            let _ = reply.send(Err("the Codex thread is not joined yet".into()));
            return;
        };
        let (method, params) = match &op {
            CodexOp::Review { target, delivery } => {
                let mut p = json!({ "threadId": tid, "target": target });
                if let Some(d) = delivery {
                    p["delivery"] = json!(d);
                }
                ("review/start", p)
            }
            CodexOp::SetGoal {
                objective,
                status,
                token_budget,
            } => {
                let mut p = json!({ "threadId": tid });
                if let Some(o) = objective {
                    p["objective"] = json!(o);
                }
                if let Some(s) = status {
                    p["status"] = json!(s);
                }
                if let Some(b) = token_budget {
                    p["tokenBudget"] = json!(b);
                }
                ("thread/goal/set", p)
            }
            CodexOp::ClearGoal => ("thread/goal/clear", json!({ "threadId": tid })),
            CodexOp::SetThreadName { name } => {
                ("thread/name/set", json!({ "threadId": tid, "name": name }))
            }
            CodexOp::WindowsSandboxSetup { .. } => unreachable!("handled above"),
        };
        self.send(
            out_tx,
            method,
            params,
            Inflight::Control { method, op, reply },
        );
    }
}

fn append_log(t: &mut Terminal, delta: &str, session_total: &mut u64) {
    let Some(path) = &t.log else {
        return;
    };
    if t.logged >= MAX_LOG || *session_total >= MAX_SESSION_LOGS {
        return;
    }
    if super::codex_terminal_logs::append(path, delta.as_bytes()).is_ok() {
        t.logged += delta.len() as u64;
        *session_total += delta.len() as u64;
    }
}

/// Seed the shared plan-mode state with the spawn's model and effort.
pub(super) fn collaboration_seed(
    model: Option<&str>,
    effort: Option<&str>,
) -> Arc<CollaborationShared> {
    let shared = CollaborationShared::default();
    *shared.model.lock().unwrap() = model.map(str::to_owned);
    *shared.effort.lock().unwrap() = effort.map(str::to_owned);
    Arc::new(shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::codex_controls::CodexControlHandle;
    use crate::session::conversation::ConversationItem;

    type Fixture = (
        SessionStore,
        ConversationStore,
        Extras,
        mpsc::UnboundedReceiver<Value>,
        mpsc::UnboundedSender<Value>,
        PathBuf,
    );

    fn fixture(experimental: bool) -> Fixture {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed("s", "/work", "codex");
        let logs = std::env::temp_dir()
            .join("claudemon-codex-extras-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let extras = Extras::new(
            "s",
            "/work",
            experimental,
            collaboration_seed(Some("gpt-5.5"), Some("high")),
        )
        .with_platform(false, logs.clone());
        let (tx, rx) = mpsc::unbounded_channel();
        (store, conv, extras, rx, tx, logs)
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<Value>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(v) = rx.try_recv() {
            out.push(v);
        }
        out
    }

    fn notices(conv: &ConversationStore) -> Vec<String> {
        conv.snapshot("s")
            .map(|(_, items)| items)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| match i {
                ConversationItem::AssistantNotice { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn review_items_become_notices_and_the_findings_assistant_text() {
        let entered = translate(
            "item/started",
            &json!({"item":{"type":"enteredReviewMode","id":"t9","review":"current changes"}}),
        );
        assert!(
            matches!(&entered[..], [AgentUpdate::Notice(t)] if t == "Codex review started: current changes")
        );
        // The started half of exitedReviewMode carries nothing to render.
        assert!(translate(
            "item/started",
            &json!({"item":{"type":"exitedReviewMode","id":"t9","review":"x"}})
        )
        .is_empty());
        let exited = translate(
            "item/completed",
            &json!({"item":{"type":"exitedReviewMode","id":"t9","review":"Looks solid.\n\n- Prefer X — app.rs:10"}}),
        );
        assert!(
            matches!(&exited[..], [AgentUpdate::Notice(n), AgentUpdate::AssistantText(t)]
            if n == "Codex review finished" && t.starts_with("Looks solid."))
        );
    }

    #[test]
    fn a_completed_plan_item_is_rendered_and_deltas_are_not() {
        let done = translate(
            "item/completed",
            &json!({"item":{"type":"plan","id":"p","text":"1. Read\n2. Fix"}}),
        );
        assert!(
            matches!(&done[..], [AgentUpdate::AssistantText(t)] if t.contains("**Proposed plan**") && t.ends_with("2. Fix"))
        );
        assert!(translate("item/plan/delta", &json!({"itemId":"p","delta":"1."})).is_empty());
        assert!(translate(
            "item/started",
            &json!({"item":{"type":"plan","id":"p","text":""}})
        )
        .is_empty());
        // codex::translate carries it too, and does not double it as a checklist.
        let via_driver = super::super::codex::translate(
            "item/completed",
            &json!({"item":{"type":"plan","id":"p","text":"do it"}}),
        );
        assert_eq!(via_driver.len(), 1);
    }

    #[test]
    fn plan_mode_stamps_collaboration_mode_only_once_chosen() {
        let c = collaboration_seed(Some("gpt-5.5"), Some("high"));
        let mut params = json!({"threadId":"t"});
        stamp_turn(&mut params, Some(&c));
        assert!(
            params.get("collaborationMode").is_none(),
            "unavailable: untouched"
        );
        c.available.store(true, Ordering::Relaxed);
        stamp_turn(&mut params, Some(&c));
        assert!(
            params.get("collaborationMode").is_none(),
            "never chosen: untouched"
        );
        *c.plan_effort.lock().unwrap() = Some(json!("medium"));
        c.set_plan(true);
        stamp_turn(&mut params, Some(&c));
        assert_eq!(
            params["collaborationMode"],
            json!({"mode":"plan","settings":{"model":"gpt-5.5","reasoning_effort":"medium","developer_instructions":null}})
        );
        // Leaving plan mode says so explicitly, with the thread's own effort.
        c.set_plan(false);
        let mut next = json!({});
        stamp_turn(&mut next, Some(&c));
        assert_eq!(next["collaborationMode"]["mode"], "default");
        assert_eq!(
            next["collaborationMode"]["settings"]["reasoning_effort"],
            "high"
        );
        // No model known: the turn goes without the mode rather than be refused.
        let blind = collaboration_seed(None, None);
        blind.available.store(true, Ordering::Relaxed);
        blind.set_plan(true);
        let mut p = json!({});
        stamp_turn(&mut p, Some(&blind));
        assert!(p.get("collaborationMode").is_none());
    }

    #[test]
    fn initialize_capabilities_and_startup_follow_the_opt_in() {
        let (_s, _c, mut off, mut rx, tx, _) = fixture(false);
        assert_eq!(off.initialize_capabilities(), None);
        off.startup(&tx);
        assert!(
            drain(&mut rx).is_empty(),
            "no opt-in, not Windows: nothing sent"
        );
        let (_s, _c, on, mut rx, tx, logs) = fixture(true);
        let mut on = on.with_platform(true, logs);
        assert_eq!(
            on.initialize_capabilities(),
            Some(json!({"experimentalApi":true}))
        );
        on.startup(&tx);
        let sent: Vec<String> = drain(&mut rx)
            .iter()
            .map(|v| v["method"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(sent, ["windowsSandbox/readiness", "collaborationMode/list"]);
    }

    #[test]
    fn collaboration_list_enables_plan_mode_and_permission_mode_switches_it() {
        let (store, conv, mut x, mut rx, tx, _) = fixture(true);
        let (ctl, _ctl_rx) = mpsc::unbounded_channel();
        let (prompts, _prompts_rx) = mpsc::unbounded_channel();
        store.register_managed_prompt("s", prompts);
        store.register_managed_yolo(
            "s",
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            false,
        );
        store.register_codex_control(
            "s",
            CodexControlHandle {
                tx: ctl,
                collaboration: x.collaboration.clone(),
            },
        );
        assert_eq!(
            store.set_managed_permission_mode("s", "plan"),
            Err(crate::session::PermissionSwitchError::PlanUnavailable)
        );
        x.startup(&tx);
        let list = drain(&mut rx).pop().unwrap();
        assert!(x.intercept(
            &json!({"id": list["id"], "result": {"data":[
                {"name":"Default","mode":"default","model":null,"reasoning_effort":null},
                {"name":"Plan","mode":"plan","model":null,"reasoning_effort":"medium"}]}}),
            &store,
            &conv,
            &tx,
            None
        ));
        assert_eq!(
            store.get("s").unwrap().collaboration_mode.as_deref(),
            Some("default")
        );
        // `plan` answers with the approval posture it kept.
        assert_eq!(store.set_managed_permission_mode("s", "plan"), Ok("ask"));
        assert!(x.collaboration.is_plan());
        assert_eq!(
            store.get("s").unwrap().collaboration_mode.as_deref(),
            Some("plan")
        );
        assert_eq!(store.set_managed_permission_mode("s", "ask"), Ok("ask"));
        assert!(!x.collaboration.is_plan());
        assert_eq!(
            store.get("s").unwrap().collaboration_mode.as_deref(),
            Some("default")
        );
    }

    #[test]
    fn a_build_without_the_methods_degrades_every_probe_quietly() {
        let (store, conv, x, mut rx, tx, logs) = fixture(true);
        let mut x = x.with_platform(true, logs);
        x.startup(&tx);
        x.observe(
            &json!({"method":"turn/started","params":{}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        // Real Codex's refusal of a method it does not know: -32600 with
        // its whole method list.
        let list: Vec<String> = (0..400).map(|i| format!("`method/{i}`")).collect();
        let not_found = json!({"code":-32600,"message":format!(
            "Invalid request: unknown variant `thread/goal/get`, expected one of {}",
            list.join(", "))});
        let probes = drain(&mut rx);
        assert_eq!(probes.len(), 3, "readiness, collaboration list, goal get");
        for request in probes {
            assert!(x.intercept(
                &json!({"id": request["id"], "error": not_found}),
                &store,
                &conv,
                &tx,
                Some("t")
            ));
        }
        let row = store.get("s").unwrap();
        assert!(
            row.goal.is_none() && row.windows_sandbox.is_none() && row.collaboration_mode.is_none()
        );
        assert!(!x.collaboration.is_available());
        assert!(notices(&conv).is_empty(), "a missing feature is not news");
        // A user action on a missing method answers with the reason.
        let (reply, mut verdict) = oneshot::channel();
        x.control(
            CodexControl {
                op: CodexOp::ClearGoal,
                reply,
            },
            &tx,
            Some("t"),
        );
        let req = drain(&mut rx).pop().unwrap();
        assert_eq!(req["method"], "thread/goal/clear");
        x.intercept(
            &json!({"id": req["id"], "error": not_found}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        assert_eq!(
            verdict.try_recv().unwrap(),
            Err("this Codex build does not support thread/goal/clear".to_string())
        );
        // Our responses are always consumed; anything else never is.
        assert!(!x.intercept(
            &json!({"id": 3, "result": {}}),
            &store,
            &conv,
            &tx,
            Some("t")
        ));
        assert!(!x.intercept(
            &json!({"id":"workspacer-skill-roots","result":{}}),
            &store,
            &conv,
            &tx,
            Some("t")
        ));
        // Without the opt-in an experimental method's refusal names it.
        assert_eq!(
            rpc_error(
                "collaborationMode/list",
                &json!({"code":-32600,"message":"collaborationMode/list requires experimentalApi capability"})
            ),
            "collaborationMode/list needs the codex.experimentalApi opt-in"
        );
    }

    #[test]
    fn goals_load_on_subscribe_follow_notifications_and_set_or_clear() {
        let (store, conv, mut x, mut rx, tx, _) = fixture(false);
        x.observe(
            &json!({"method":"turn/started","params":{}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            false,
        );
        assert!(
            drain(&mut rx).is_empty(),
            "nothing before the thread is joined"
        );
        x.observe(
            &json!({"id":2,"result":{"model":"gpt-5.4","reasoningEffort":"low","thread":{"id":"t","name":"Fix login"}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        let get = drain(&mut rx).pop().unwrap();
        assert_eq!(get["method"], "thread/goal/get");
        assert_eq!(get["params"], json!({"threadId":"t"}));
        assert_eq!(
            store.get("s").unwrap().thread_name.as_deref(),
            Some("Fix login")
        );
        assert_eq!(
            x.collaboration.model.lock().unwrap().as_deref(),
            Some("gpt-5.4")
        );
        x.intercept(
            &json!({"id":get["id"],"result":{"goal":{"threadId":"t","objective":"Ship v2","status":"active","tokenBudget":null,"tokensUsed":10,"timeUsedSeconds":3,"createdAt":1,"updatedAt":2}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        let goal = store.get("s").unwrap().goal.unwrap();
        assert_eq!(
            (
                goal.objective.as_str(),
                goal.status.as_str(),
                goal.token_budget
            ),
            ("Ship v2", "active", None)
        );
        assert_eq!(
            serde_json::to_value(&goal).unwrap(),
            json!({"objective":"Ship v2","status":"active","tokensUsed":10,"timeUsedSeconds":3})
        );
        x.observe(
            &json!({"method":"thread/goal/updated","params":{"threadId":"t","turnId":null,"goal":{"objective":"Ship v2","status":"budgetLimited","tokenBudget":500}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert_eq!(
            store.get("s").unwrap().goal.unwrap().status,
            "budgetLimited"
        );
        // Another thread's (a subagent's) goal is not ours.
        x.observe(
            &json!({"method":"thread/goal/cleared","params":{"threadId":"child"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert!(store.get("s").unwrap().goal.is_some());
        x.observe(
            &json!({"method":"thread/goal/cleared","params":{"threadId":"t"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert!(store.get("s").unwrap().goal.is_none());

        let (reply, mut verdict) = oneshot::channel();
        x.control(
            CodexControl {
                op: CodexOp::SetGoal {
                    objective: Some("Do it".into()),
                    status: None,
                    token_budget: Some(None),
                },
                reply,
            },
            &tx,
            Some("t"),
        );
        let set = drain(&mut rx).pop().unwrap();
        assert_eq!(set["method"], "thread/goal/set");
        assert_eq!(
            set["params"],
            json!({"threadId":"t","objective":"Do it","tokenBudget":null})
        );
        x.intercept(
            &json!({"id":set["id"],"result":{"goal":{"objective":"Do it","status":"active","tokenBudget":null}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        assert!(verdict.try_recv().unwrap().is_ok());
        assert_eq!(store.get("s").unwrap().goal.unwrap().objective, "Do it");
        // Without a thread, a control is refused rather than queued forever.
        let (reply, mut verdict) = oneshot::channel();
        x.control(
            CodexControl {
                op: CodexOp::ClearGoal,
                reply,
            },
            &tx,
            None,
        );
        assert!(verdict.try_recv().unwrap().is_err());
    }

    #[test]
    fn review_and_rename_send_codex_shapes() {
        let (store, conv, mut x, mut rx, tx, _) = fixture(false);
        let (reply, mut verdict) = oneshot::channel();
        x.control(
            CodexControl {
                op: CodexOp::Review {
                    target: json!({"type":"baseBranch","branch":"main"}),
                    delivery: None,
                },
                reply,
            },
            &tx,
            Some("t"),
        );
        let review = drain(&mut rx).pop().unwrap();
        assert_eq!(review["method"], "review/start");
        assert_eq!(
            review["params"],
            json!({"threadId":"t","target":{"type":"baseBranch","branch":"main"}})
        );
        x.intercept(
            &json!({"id":review["id"],"result":{"turn":{"id":"turn_9"},"reviewThreadId":"t"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        assert_eq!(verdict.try_recv().unwrap().unwrap()["reviewThreadId"], "t");

        let (reply, mut verdict) = oneshot::channel();
        x.control(
            CodexControl {
                op: CodexOp::SetThreadName {
                    name: "Auth fix".into(),
                },
                reply,
            },
            &tx,
            Some("t"),
        );
        let name = drain(&mut rx).pop().unwrap();
        assert_eq!(name["params"], json!({"threadId":"t","name":"Auth fix"}));
        x.intercept(
            &json!({"id":name["id"],"result":{}}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        assert!(verdict.try_recv().unwrap().is_ok());
        assert_eq!(
            store.get("s").unwrap().thread_name.as_deref(),
            Some("Auth fix")
        );
        // The TUI's /rename (or its automatic title) arrives as a notification.
        x.observe(
            &json!({"method":"thread/name/updated","params":{"threadId":"t","threadName":"Renamed in TUI"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert_eq!(
            store.get("s").unwrap().thread_name.as_deref(),
            Some("Renamed in TUI")
        );
    }

    #[test]
    fn windows_sandbox_readiness_warnings_and_setup() {
        let (store, conv, x, mut rx, tx, logs) = fixture(false);
        let mut x = x.with_platform(true, logs);
        x.startup(&tx);
        let ready = drain(&mut rx).pop().unwrap();
        assert_eq!(ready["method"], "windowsSandbox/readiness");
        assert!(ready.get("params").is_none(), "readiness takes no params");
        x.intercept(
            &json!({"id":ready["id"],"result":{"status":"notConfigured"}}),
            &store,
            &conv,
            &tx,
            None,
        );
        let ws = store.get("s").unwrap().windows_sandbox.unwrap();
        assert_eq!((ws.ready, ws.status.as_str()), (false, "notConfigured"));
        assert!(notices(&conv)[0].contains("is not set up"));

        let (reply, mut verdict) = oneshot::channel();
        x.control(
            CodexControl {
                op: CodexOp::WindowsSandboxSetup {
                    mode: "elevated".into(),
                },
                reply,
            },
            &tx,
            None,
        );
        let setup = drain(&mut rx).pop().unwrap();
        assert_eq!(setup["params"], json!({"mode":"elevated","cwd":"/work"}));
        x.intercept(
            &json!({"id":setup["id"],"result":{"started":true}}),
            &store,
            &conv,
            &tx,
            None,
        );
        assert_eq!(
            verdict.try_recv().unwrap().unwrap(),
            json!({"started":true})
        );
        assert_eq!(
            store.get("s").unwrap().windows_sandbox.unwrap().status,
            "settingUp"
        );
        x.observe(
            &json!({"method":"windowsSandbox/setupCompleted","params":{"mode":"elevated","success":true,"error":null}}),
            &store,
            &conv,
            &tx,
            None,
            false,
        );
        let again = drain(&mut rx).pop().unwrap();
        assert_eq!(again["method"], "windowsSandbox/readiness");
        x.intercept(
            &json!({"id":again["id"],"result":{"status":"ready"}}),
            &store,
            &conv,
            &tx,
            None,
        );
        assert!(store.get("s").unwrap().windows_sandbox.unwrap().ready);
        assert!(notices(&conv).last().unwrap().contains("is ready"));

        x.observe(
            &json!({"method":"windows/worldWritableWarning","params":{"samplePaths":["C:\\Temp"],"extraCount":2,"failedScan":false}}),
            &store,
            &conv,
            &tx,
            None,
            false,
        );
        let ws = store.get("s").unwrap().windows_sandbox.unwrap();
        assert!(ws.detail.unwrap().contains("C:\\Temp (+2 more)"));

        // Not Windows: setup is refused here, never sent.
        let (store2, _c, mut other, mut rx2, tx2, _) = fixture(false);
        let (reply, mut verdict) = oneshot::channel();
        other.control(
            CodexControl {
                op: CodexOp::WindowsSandboxSetup {
                    mode: "elevated".into(),
                },
                reply,
            },
            &tx2,
            Some("t"),
        );
        assert!(drain(&mut rx2).is_empty());
        assert!(verdict
            .try_recv()
            .unwrap()
            .unwrap_err()
            .contains("only to Codex running on Windows"));
        assert!(store2.get("s").unwrap().windows_sandbox.is_none());
    }

    fn command_item(id: &str, process: &str, source: &str) -> Value {
        json!({"threadId":"t","turnId":"u1","item":{"type":"commandExecution","id":id,"command":"npm run dev","cwd":"/work","processId":process,"source":source,"status":"inProgress","commandActions":[]}})
    }

    #[test]
    fn a_terminal_still_running_at_turn_end_becomes_a_background_task_with_a_log() {
        let (store, conv, mut x, mut rx, tx, logs) = fixture(false);
        let mode = store.get("s").unwrap().mode;
        let obs = |x: &mut Extras, v: Value| x.observe(&v, &store, &conv, &tx, Some("t"), true);
        // A short command finishes inside its turn: never a background task.
        obs(
            &mut x,
            json!({"method":"item/started","params":command_item("quick","7","unifiedExecStartup")}),
        );
        obs(
            &mut x,
            json!({"method":"item/completed","params":{"threadId":"t","item":{"type":"commandExecution","id":"quick","status":"completed","exitCode":0}}}),
        );
        obs(
            &mut x,
            json!({"method":"item/started","params":command_item("dev","42","unifiedExecStartup")}),
        );
        obs(
            &mut x,
            json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"t","turnId":"u1","itemId":"dev","delta":"listening on :3000\n"}}),
        );
        assert!(store.get("s").unwrap().background_task_list.is_empty());
        obs(
            &mut x,
            json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u1","status":"completed"}}}),
        );
        let row = store.get("s").unwrap();
        assert_eq!(row.background_tasks, 1);
        let task = &row.background_task_list[0];
        assert_eq!(
            (
                task.id.as_str(),
                task.task_type.as_str(),
                task.status.as_str()
            ),
            ("codex-42", "local_bash", "running")
        );
        assert_eq!(task.description.as_deref(), Some("npm run dev"));
        assert!(task.has_output);
        let wire = serde_json::to_value(task).unwrap();
        assert!(
            wire.get("outputFile").is_none() && wire.get("output_file").is_none(),
            "no path on the wire"
        );
        obs(
            &mut x,
            json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"t","itemId":"dev","delta":"GET / 200\n"}}),
        );
        let log = bg::read_output(task.output_file.as_ref().unwrap(), Some(0), 1024).unwrap();
        assert_eq!(log.text, "listening on :3000\nGET / 200\n");
        assert!(
            drain(&mut rx)
                .iter()
                .all(|v| v["method"] != "thread/backgroundTerminals/list"),
            "list is experimental"
        );
        // The process exits later; the row closes with its exit code.
        obs(
            &mut x,
            json!({"method":"item/completed","params":{"threadId":"t","item":{"type":"commandExecution","id":"dev","status":"failed","exitCode":130}}}),
        );
        let row = store.get("s").unwrap();
        assert_eq!(row.background_tasks, 0);
        assert_eq!(row.background_task_list[0].status, "failed");
        assert_eq!(
            row.background_task_list[0].summary.as_deref(),
            Some("exit code 130")
        );
        assert_eq!(row.mode, mode, "background terminals never move the mode");
        let _ = std::fs::remove_dir_all(logs);
    }

    #[test]
    fn stdin_interaction_promotes_and_stop_needs_the_opt_in() {
        let (store, conv, mut x, mut rx, tx, logs) = fixture(false);
        x.observe(
            &json!({"method":"item/started","params":command_item("py","9","unifiedExecStartup")}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        x.observe(
            &json!({"method":"item/commandExecution/terminalInteraction","params":{"threadId":"t","turnId":"u1","itemId":"py","processId":"9","stdin":"print(1)\n"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert_eq!(
            store.get("s").unwrap().background_task_list[0].id,
            "codex-9"
        );
        drain(&mut rx);
        let (reply, mut verdict) = oneshot::channel();
        x.stop_task(
            ManagedTaskStop {
                task_id: "codex-9".into(),
                reply,
            },
            &tx,
            Some("t"),
        );
        assert!(verdict
            .try_recv()
            .unwrap()
            .unwrap_err()
            .contains("codex.experimentalApi"));
        assert!(drain(&mut rx).is_empty());
        let _ = std::fs::remove_dir_all(logs);
    }

    #[test]
    fn experimental_list_reconciles_and_terminate_stops_a_terminal() {
        let (store, conv, mut x, mut rx, tx, logs) = fixture(true);
        x.observe(
            &json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u1","status":"completed"}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        let list = drain(&mut rx)
            .into_iter()
            .find(|v| v["method"] == "thread/backgroundTerminals/list")
            .unwrap();
        assert_eq!(list["params"], json!({"threadId":"t"}));
        x.intercept(
            &json!({"id":list["id"],"result":{"data":[{"itemId":"old","processId":"5","command":"tail -f log","cwd":"/work","osPid":null,"cpuPercent":null,"rssKb":null}],"nextCursor":null}}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        let row = store.get("s").unwrap();
        assert_eq!(row.background_task_list[0].id, "codex-5");
        assert_eq!(
            row.background_task_list[0].description.as_deref(),
            Some("tail -f log")
        );

        let (reply, mut verdict) = oneshot::channel();
        x.stop_task(
            ManagedTaskStop {
                task_id: "codex-5".into(),
                reply,
            },
            &tx,
            Some("t"),
        );
        let term = drain(&mut rx).pop().unwrap();
        assert_eq!(term["method"], "thread/backgroundTerminals/terminate");
        assert_eq!(term["params"], json!({"threadId":"t","processId":"5"}));
        x.intercept(
            &json!({"id":term["id"],"result":{"terminated":true}}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        assert_eq!(verdict.try_recv().unwrap(), Ok(()));
        let row = store.get("s").unwrap();
        assert_eq!(
            (
                row.background_task_list[0].status.as_str(),
                row.background_tasks
            ),
            ("stopped", 0)
        );
        // A late failed item/completed does not resurrect or rewrite it.
        x.observe(
            &json!({"method":"item/completed","params":{"threadId":"t","item":{"type":"commandExecution","id":"old","status":"failed","exitCode":143}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert_eq!(
            store.get("s").unwrap().background_task_list[0].status,
            "stopped"
        );

        // A build without the method: the stop answers why.
        x.observe(
            &json!({"method":"item/started","params":command_item("b","6","unifiedExecStartup")}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        x.observe(
            &json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u2"}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        drain(&mut rx);
        let (reply, mut verdict) = oneshot::channel();
        x.stop_task(
            ManagedTaskStop {
                task_id: "codex-6".into(),
                reply,
            },
            &tx,
            Some("t"),
        );
        let term = drain(&mut rx).pop().unwrap();
        x.intercept(
            &json!({"id":term["id"],"error":{"code":-32601,"message":"Method not found"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
        );
        assert_eq!(
            verdict.try_recv().unwrap(),
            Err("this Codex build does not support thread/backgroundTerminals/terminate".into())
        );
        assert_eq!(store.get("s").unwrap().background_tasks, 1, "still running");
        let _ = std::fs::remove_dir_all(logs);
    }

    #[test]
    fn terminal_task_ids_are_plain_segments() {
        assert_eq!(terminal_task_id("42", "call_1"), "codex-42");
        let odd = terminal_task_id("a/b c", "call:xyz");
        assert!(bg::valid_task_id(&odd), "{odd}");
        assert_eq!(odd, "codex-callxyz");
    }
}

/// End to end through `codex::run_session` against a scripted stand-in
/// app-server: the extras ride the real driver loop, and a server that knows
/// none of their methods (answering the way real Codex does) leaves the chat
/// path untouched.
#[cfg(all(test, unix))]
mod driver_tests {
    use super::super::codex::{run_session, DriverEvidence};
    use super::super::{Facade, SpawnExtras};
    use crate::session::codex_controls::{CodexControl, CodexOp};
    use crate::session::conversation::ConversationItem;
    use crate::session::state::SessionMode;
    use crate::session::{ConversationStore, SessionStore};
    use serde_json::{json, Value};

    const FAKE: &str = r#"#!/usr/bin/env python3
import base64, hashlib, http.server, json, os, struct, sys, time
script = json.load(open(os.environ['WKS_FAKE_SCRIPT']))
port = int(sys.argv[sys.argv.index('--listen') + 1].rsplit(':', 1)[1])
class Server(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args): pass
    def do_GET(self):
        if self.path == '/readyz':
            self.send_response(200); self.send_header('Content-Length', '0'); self.end_headers(); return
        key = self.headers['Sec-WebSocket-Key']
        digest = hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()
        self.send_response(101)
        self.send_header('Upgrade', 'websocket')
        self.send_header('Connection', 'Upgrade')
        self.send_header('Sec-WebSocket-Accept', base64.b64encode(digest).decode())
        self.end_headers()
        def send(value):
            body = json.dumps(value).encode()
            if len(body) < 126: length = bytes([len(body)])
            elif len(body) < 65536: length = b'\x7e' + struct.pack('!H', len(body))
            else: length = b'\x7f' + struct.pack('!Q', len(body))
            self.connection.sendall(b'\x81' + length + body)
        def recv():
            head = self.rfile.read(2)
            if len(head) < 2: return None
            size = head[1] & 0x7f
            if size == 126: size = struct.unpack('!H', self.rfile.read(2))[0]
            elif size == 127: size = struct.unpack('!Q', self.rfile.read(8))[0]
            mask = self.rfile.read(4) if head[1] & 0x80 else b'\0\0\0\0'
            data = bytearray(self.rfile.read(size))
            for i in range(size): data[i] ^= mask[i % 4]
            if head[0] & 0x0f == 8: return None
            return data.decode() if head[0] & 0x0f == 1 else ''
        log = open(os.environ['WKS_FAKE_DIR'] + '/requests', 'a')
        turns = 0
        while True:
            text = recv()
            if text is None: return
            for line in text.split('\n'):
                if not line.strip(): continue
                msg = json.loads(line)
                log.write(line + '\n'); log.flush()
                method = msg.get('method')
                if 'id' not in msg or method is None: continue
                if method in script.get('unsupported', []):
                    known = ', '.join('`m/%d`' % i for i in range(300))
                    send({'id': msg['id'], 'error': {'code': -32600, 'message': 'Invalid request: unknown variant `%s`, expected one of %s' % (method, known)}})
                    continue
                if method == 'turn/start':
                    turns += 1
                    turn = {'id': 'turn-%d' % turns, 'items': [], 'status': 'inProgress'}
                    send({'id': msg['id'], 'result': {'turn': turn}})
                    send({'method': 'turn/started', 'params': {'threadId': 'fixture-thread', 'turn': turn}})
                    for n in script.get('turn', []): send(n)
                    send({'method': 'turn/completed', 'params': {'threadId': 'fixture-thread', 'turn': dict(turn, status='completed')}})
                    continue
                send({'id': msg['id'], 'result': script.get('results', {}).get(method, {})})
                for n in script.get('after', {}).get(method, []): send(n)
http.server.HTTPServer(('127.0.0.1', port), Server).serve_forever()
"#;

    struct Fake {
        dir: std::path::PathBuf,
        bin: String,
        extras: SpawnExtras,
    }

    impl Fake {
        fn new(script: Value, experimental: bool) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!("codex-extras-fixture-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let bin = dir.join("fake-codex");
            std::fs::write(&bin, FAKE).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
            let script_path = dir.join("script.json");
            std::fs::write(&script_path, script.to_string()).unwrap();
            let mut extras = SpawnExtras {
                codex_experimental: experimental,
                ..SpawnExtras::default()
            };
            for (k, v) in [
                (
                    "WKS_FAKE_SCRIPT",
                    script_path.to_string_lossy().into_owned(),
                ),
                ("WKS_FAKE_DIR", dir.to_string_lossy().into_owned()),
            ] {
                extras.env.insert(k.into(), v);
            }
            Self {
                bin: bin.to_string_lossy().into_owned(),
                dir,
                extras,
            }
        }

        fn sent(&self) -> Vec<Value> {
            std::fs::read_to_string(self.dir.join("requests"))
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        }

        fn calls(&self, method: &str) -> Vec<Value> {
            self.sent()
                .into_iter()
                .filter(|v| v["method"] == method)
                .collect()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn until(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..600 {
            if done() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        done()
    }

    fn start(
        fake: &Fake,
        sid: &'static str,
    ) -> (SessionStore, ConversationStore, tokio::task::JoinHandle<()>) {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed(sid, fake.dir.to_str().unwrap(), "codex");
        let generation = store.claim_generation(sid);
        let (task_store, task_conv) = (store.clone(), conv.clone());
        let dir = fake.dir.to_str().unwrap().to_owned();
        let (bin, extras) = (fake.bin.clone(), fake.extras.clone());
        let task = tokio::spawn(async move {
            let mut evidence = DriverEvidence::new(sid, generation);
            let _ = run_session(
                &task_store,
                &task_conv,
                sid,
                &dir,
                Some("gpt-5.5".into()),
                Some("high".into()),
                None,
                &bin,
                false,
                true,
                Some("fixture-thread".into()),
                None,
                &Facade::default(),
                &extras,
                &mut evidence,
            )
            .await;
        });
        (store, conv, task)
    }

    async fn control(store: &SessionStore, sid: &str, op: CodexOp) -> Result<Value, String> {
        let (reply, verdict) = tokio::sync::oneshot::channel();
        store
            .codex_control(sid)
            .expect("codex control registered")
            .tx
            .send(CodexControl { op, reply })
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), verdict)
            .await
            .expect("the driver answered")
            .expect("reply sent")
    }

    fn assistant_text(conv: &ConversationStore, sid: &str) -> String {
        conv.snapshot(sid)
            .map(|(_, items)| items)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| match i {
                ConversationItem::AssistantText { text, .. } => Some(text),
                ConversationItem::AssistantNotice { text, .. } => Some(format!("[{text}]")),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[tokio::test]
    async fn experimental_codex_runs_plan_mode_review_and_goals_through_the_driver() {
        let fake = Fake::new(
            json!({
                "results": {
                    "collaborationMode/list": {"data": [
                        {"name":"Default","mode":"default","model":null,"reasoning_effort":null},
                        {"name":"Plan","mode":"plan","model":null,"reasoning_effort":"medium"}]},
                    "thread/goal/get": {"goal": {"threadId":"fixture-thread","objective":"Ship v2","status":"active","tokenBudget":null,"tokensUsed":0,"timeUsedSeconds":0,"createdAt":1,"updatedAt":1}},
                    "review/start": {"turn": {"id":"review-1","status":"inProgress","items":[]}, "reviewThreadId":"fixture-thread"},
                    "thread/goal/clear": {"cleared": true}
                },
                "after": {
                    "review/start": [
                        {"method":"turn/started","params":{"threadId":"fixture-thread","turn":{"id":"review-1"}}},
                        {"method":"item/started","params":{"threadId":"fixture-thread","item":{"type":"enteredReviewMode","id":"review-1","review":"current changes"}}},
                        {"method":"item/completed","params":{"threadId":"fixture-thread","item":{"type":"exitedReviewMode","id":"review-1","review":"No issues found."}}},
                        {"method":"turn/completed","params":{"threadId":"fixture-thread","turn":{"id":"review-1","status":"completed"}}}
                    ],
                    "thread/goal/clear": [
                        {"method":"thread/goal/cleared","params":{"threadId":"fixture-thread"}}
                    ]
                },
                "turn": [
                    {"method":"item/completed","params":{"threadId":"fixture-thread","item":{"type":"plan","id":"p1","text":"1. Read the code"}}}
                ]
            }),
            true,
        );
        let sid = "extras-experimental";
        let (store, conv, task) = start(&fake, sid);
        assert!(until(|| !fake.calls("initialize").is_empty()).await);
        assert_eq!(
            fake.calls("initialize")[0]["params"]["capabilities"],
            json!({"experimentalApi": true})
        );
        assert!(
            until(|| store.get(sid).unwrap().collaboration_mode.as_deref() == Some("default"))
                .await
        );
        assert!(until(|| store.get(sid).unwrap().goal.is_some()).await);
        assert_eq!(store.get(sid).unwrap().goal.unwrap().objective, "Ship v2");

        assert_eq!(store.set_managed_permission_mode(sid, "plan"), Ok("ask"));
        assert_eq!(
            store.get(sid).unwrap().collaboration_mode.as_deref(),
            Some("plan")
        );
        assert!(
            until(|| matches!(
                store.submit_message(sid, "plan it".into()),
                crate::session::store::MessageOutcome::Sent
            ))
            .await
        );
        assert!(until(|| !fake.calls("turn/start").is_empty()).await);
        assert_eq!(
            fake.calls("turn/start")[0]["params"]["collaborationMode"],
            json!({"mode":"plan","settings":{"model":"gpt-5.5","reasoning_effort":"medium","developer_instructions":null}})
        );
        assert!(until(|| assistant_text(&conv, sid).contains("1. Read the code")).await);
        assert!(until(|| store.get(sid).unwrap().mode == SessionMode::Input).await);

        let review = control(
            &store,
            sid,
            CodexOp::Review {
                target: json!({"type":"uncommittedChanges"}),
                delivery: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(review["reviewThreadId"], "fixture-thread");
        assert!(until(|| assistant_text(&conv, sid).contains("No issues found.")).await);
        let text = assistant_text(&conv, sid);
        assert!(
            text.contains("[Codex review started: current changes]"),
            "{text}"
        );
        assert!(text.contains("[Codex review finished]"), "{text}");

        assert_eq!(
            control(&store, sid, CodexOp::ClearGoal).await.unwrap(),
            json!({"cleared": true})
        );
        assert!(until(|| store.get(sid).unwrap().goal.is_none()).await);
        store.terminate_managed(sid);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(8), task).await;
    }

    #[tokio::test]
    async fn a_server_without_the_extras_still_chats_and_answers_why() {
        let fake = Fake::new(
            json!({
                "unsupported": [
                    "collaborationMode/list", "thread/goal/get", "thread/goal/set",
                    "thread/goal/clear", "review/start", "thread/name/set",
                    "windowsSandbox/readiness", "thread/backgroundTerminals/list",
                    "thread/backgroundTerminals/terminate"
                ],
                "turn": [
                    {"method":"item/agentMessage/delta","params":{"threadId":"fixture-thread","itemId":"m","delta":"hello"}}
                ]
            }),
            false,
        );
        let sid = "extras-missing";
        let (store, conv, task) = start(&fake, sid);
        assert!(until(|| !fake.calls("thread/goal/get").is_empty()).await);
        assert!(fake.calls("initialize")[0]["params"]
            .get("capabilities")
            .is_none());
        assert!(
            fake.calls("collaborationMode/list").is_empty(),
            "no opt-in, no probe"
        );
        assert_eq!(
            store.set_managed_permission_mode(sid, "plan"),
            Err(crate::session::PermissionSwitchError::PlanUnavailable)
        );
        assert!(
            until(|| matches!(
                store.submit_message(sid, "hi".into()),
                crate::session::store::MessageOutcome::Sent
            ))
            .await
        );
        assert!(until(|| assistant_text(&conv, sid).contains("hello")).await);
        assert!(until(|| store.get(sid).unwrap().mode == SessionMode::Input).await);
        assert!(fake.calls("turn/start")[0]["params"]
            .get("collaborationMode")
            .is_none());
        for (op, method) in [
            (
                CodexOp::Review {
                    target: json!({"type":"uncommittedChanges"}),
                    delivery: None,
                },
                "review/start",
            ),
            (
                CodexOp::SetGoal {
                    objective: Some("x".into()),
                    status: None,
                    token_budget: None,
                },
                "thread/goal/set",
            ),
            (
                CodexOp::SetThreadName { name: "n".into() },
                "thread/name/set",
            ),
        ] {
            assert_eq!(
                control(&store, sid, op).await,
                Err(format!("this Codex build does not support {method}"))
            );
        }
        let row = store.get(sid).unwrap();
        assert!(
            row.goal.is_none() && row.collaboration_mode.is_none() && row.thread_name.is_none()
        );
        assert_eq!(
            row.mode,
            SessionMode::Input,
            "nothing left the session stuck"
        );
        store.terminate_managed(sid);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(8), task).await;
    }
}

/// Plan-mode state that outlives a driver, error hygiene, notice noise and
/// log lifetime.
#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use crate::session::conversation::ConversationItem;
    use std::sync::atomic::AtomicBool;

    fn setup(experimental: bool) -> (SessionStore, ConversationStore, Extras, PathBuf) {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed("s", "/work", "codex");
        let logs = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("codex-extras-logs-{}", uuid::Uuid::new_v4()))
            .join("s");
        let extras = Extras::new(
            "s",
            "/work",
            experimental,
            collaboration_seed(Some("gpt-5.5"), Some("high")),
        )
        .with_platform(false, logs.clone());
        (store, conv, extras, logs)
    }

    fn plan_mode() -> Value {
        json!({"mode":"plan","settings":{"model":"gpt-5.5","reasoning_effort":"medium","developer_instructions":null}})
    }

    #[test]
    fn a_thread_resumed_in_plan_mode_stays_in_it_and_leaving_says_default() {
        let (store, conv, mut x, _) = setup(true);
        x.collaboration.available.store(true, Ordering::Relaxed);
        let (tx, _rx) = mpsc::unbounded_channel();
        x.observe(
            &json!({"id":2,"result":{"model":"gpt-5.5","thread":{"id":"t"},"collaborationMode":plan_mode()}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert!(x.collaboration.is_plan());
        assert_eq!(
            store.get("s").unwrap().collaboration_mode.as_deref(),
            Some("plan")
        );
        // The user picks ask: the next turn must leave plan explicitly.
        x.collaboration.set_plan(false);
        let mut params = json!({});
        stamp_turn(&mut params, Some(&x.collaboration));
        assert_eq!(params["collaborationMode"]["mode"], "default");
    }

    #[test]
    fn a_tui_side_mode_change_arrives_with_thread_settings() {
        let (store, conv, mut x, _) = setup(false);
        let (tx, _rx) = mpsc::unbounded_channel();
        let settings = |mode: Value| json!({"method":"thread/settings/updated","params":{"threadId":"t","threadSettings":{"model":"gpt-5.5","collaborationMode":mode}}});
        x.observe(&settings(plan_mode()), &store, &conv, &tx, Some("t"), true);
        assert!(x.collaboration.is_plan());
        // Without the opt-in a plan thread is still shown as such.
        assert_eq!(
            store.get("s").unwrap().collaboration_mode.as_deref(),
            Some("plan")
        );
        x.observe(
            &settings(json!({"mode":"default","settings":{"model":"gpt-5.5"}})),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert!(!x.collaboration.is_plan());
        assert!(!x.collaboration.engaged.load(Ordering::Relaxed));
        assert_eq!(store.get("s").unwrap().collaboration_mode, None);
    }

    #[test]
    fn a_late_turn_response_with_id_101_is_not_a_thread_result() {
        let (store, conv, mut x, _) = setup(false);
        let (tx, _rx) = mpsc::unbounded_channel();
        let started =
            json!({"id":101,"result":{"model":"gpt-5.5","thread":{"id":"t","name":"First"}}});
        x.observe(&started, &store, &conv, &tx, Some("t"), true);
        assert_eq!(
            store.get("s").unwrap().thread_name.as_deref(),
            Some("First")
        );
        // Ours again later — never re-applied; a turn result is never one.
        x.observe(
            &json!({"id":101,"result":{"model":"other","thread":{"id":"t","name":"Second"}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        x.observe(
            &json!({"id":2,"result":{"turn":{"id":"u"}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        assert_eq!(
            store.get("s").unwrap().thread_name.as_deref(),
            Some("First")
        );
        assert_eq!(
            x.collaboration.model.lock().unwrap().as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn relayed_codex_errors_are_bounded() {
        let long = "x".repeat(5_000);
        let text = rpc_error("thread/goal/set", &json!({"code":-32603,"message":long}));
        assert!(text.len() < MAX_ERROR_TEXT + 64, "{}", text.len());
        assert!(text.ends_with('…'));
        assert_eq!(
            rpc_error(
                "review/start",
                &json!({"code":-32600,"message":"Invalid request: unknown variant `review/start`, expected one of `initialize`"})
            ),
            "this Codex build does not support review/start"
        );
    }

    fn notices(conv: &ConversationStore) -> usize {
        conv.snapshot("s")
            .map(|(_, items)| items)
            .unwrap_or_default()
            .into_iter()
            .filter(|i| matches!(i, ConversationItem::AssistantNotice { .. }))
            .count()
    }

    #[test]
    fn sandbox_notices_only_for_sandboxed_sessions_and_once() {
        let (store, conv, x, logs) = setup(false);
        let mut x = x.with_platform(true, logs);
        let yolo = Arc::new(AtomicBool::new(true));
        x.watch_yolo(yolo.clone());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let answer = |x: &mut Extras, rx: &mut mpsc::UnboundedReceiver<Value>, status: &str| {
            x.startup(&tx);
            let req = rx.try_recv().unwrap();
            x.intercept(
                &json!({"id":req["id"],"result":{"status":status}}),
                &store,
                &conv,
                &tx,
                None,
            );
        };
        answer(&mut x, &mut rx, "notConfigured");
        assert_eq!(notices(&conv), 0, "bypassed: not news");
        assert!(!store.get("s").unwrap().windows_sandbox.unwrap().ready);
        let warn = json!({"method":"windows/worldWritableWarning","params":{"samplePaths":["C:\\\\T"],"extraCount":0,"failedScan":false}});
        x.observe(&warn, &store, &conv, &tx, None, false);
        assert_eq!(notices(&conv), 0);
        yolo.store(false, Ordering::Relaxed);
        store.update_state("s", |s| {
            s.windows_sandbox = None;
            true
        });
        answer(&mut x, &mut rx, "notConfigured");
        assert_eq!(notices(&conv), 1);
        store.update_state("s", |s| {
            s.windows_sandbox = None;
            true
        });
        answer(&mut x, &mut rx, "updateRequired");
        assert_eq!(notices(&conv), 1, "at most once per session");
        x.observe(&warn, &store, &conv, &tx, None, false);
        x.observe(&warn, &store, &conv, &tx, None, false);
        assert_eq!(notices(&conv), 2);
    }

    fn started(id: &str, process: &str) -> Value {
        json!({"method":"item/started","params":{"threadId":"t","item":{"type":"commandExecution","id":id,"command":"run","processId":process,"source":"unifiedExecStartup","status":"inProgress"}}})
    }

    #[test]
    fn logs_leave_with_their_row_and_with_the_driver() {
        let (store, conv, mut x, logs) = setup(false);
        let (tx, _rx) = mpsc::unbounded_channel();
        let obs = |x: &mut Extras, v: Value| x.observe(&v, &store, &conv, &tx, Some("t"), true);
        // More finished terminals than the row cap: the oldest rows go, and
        // their logs with them.
        for n in 0..(bg::MAX_TASKS + 3) {
            let id = format!("i{n}");
            obs(&mut x, started(&id, &n.to_string()));
            obs(
                &mut x,
                json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":id}}}),
            );
            obs(
                &mut x,
                json!({"method":"item/completed","params":{"threadId":"t","item":{"type":"commandExecution","id":id,"status":"completed","exitCode":0}}}),
            );
        }
        let rows = store.get("s").unwrap().background_task_list;
        assert_eq!(rows.len(), bg::MAX_TASKS);
        let files = std::fs::read_dir(&logs).unwrap().count();
        assert_eq!(files, bg::MAX_TASKS, "evicted rows' logs removed");
        drop(x);
        assert!(!logs.exists(), "the driver's end removes its logs");
        let _ = std::fs::remove_dir_all(logs.parent().unwrap());
    }

    #[test]
    fn a_session_s_logs_are_capped_in_total() {
        let (store, conv, mut x, logs) = setup(false);
        let (tx, _rx) = mpsc::unbounded_channel();
        x.logged_total = MAX_SESSION_LOGS - 4;
        x.observe(&started("a", "1"), &store, &conv, &tx, Some("t"), true);
        x.observe(
            &json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"t","itemId":"a","delta":"0123456789"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        x.observe(
            &json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u"}}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        x.observe(
            &json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"t","itemId":"a","delta":"more"}}),
            &store,
            &conv,
            &tx,
            Some("t"),
            true,
        );
        let log = logs.join("codex-1.output");
        assert_eq!(std::fs::read(&log).unwrap(), b"0123");
        drop(x);
        let _ = std::fs::remove_dir_all(logs.parent().unwrap());
    }
}
