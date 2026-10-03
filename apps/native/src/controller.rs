//! A single background reducer owns network state. The UI receives coalesced,
//! immutable views; it never parses JSON or waits for I/O during rendering.
use anyhow::{Result, anyhow};
use futures_util::{
    StreamExt,
    future::{AbortHandle, Abortable, BoxFuture},
    stream::FuturesUnordered,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;

use crate::{
    backend::Backend,
    bus::{Config, Event},
    launch::{Catalog, CatalogKey, Permission, absolute_directory, parse_models},
    model::{ConversationSnapshot, Delta, Fold, Session, Transcript},
};

const MAX_SESSIONS: usize = 10_000;
const FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// Conversation items per page: opening a session reads only the newest page,
/// and each "earlier messages" step widens the window by one more.
pub const CONVERSATION_PAGE: usize = 200;

#[derive(Clone, Debug)]
pub enum Action {
    Send(String),
    Approve(bool),
    Stop,
    Answer(String),
    Answers(Vec<String>),
    Terminate,
    SetModel {
        model: String,
        context_window: Option<u64>,
    },
}

impl Action {
    pub(crate) fn wire(&self, id: &str) -> (&'static str, Value) {
        match self {
            Self::Terminate => ("claude.signal", json!({"sessionId":id,"signal":"SIGTERM"})),
            Self::Answers(answers) => (
                "claude.answer",
                json!({"sessionId":id,"answers":answers,"answerKinds":vec!["text"; answers.len()]}),
            ),
            Self::SetModel {
                model,
                context_window,
            } => (
                "claude.setModel",
                json!({"sessionId":id,"model":model,"modelIdentity":model,"contextWindow":context_window}),
            ),
            Self::Send(text) => ("agents.sendMessage", json!({"sessionId":id, "text":text})),
            Self::Approve(yes) => (
                "claude.approve",
                json!({"sessionId":id, "decision":if *yes {"yes"} else {"no"}}),
            ),
            Self::Stop => ("claude.signal", json!({"sessionId":id, "signal":"SIGINT"})),
            Self::Answer(text) => (
                "claude.answer",
                json!({"sessionId":id, "text":text,"answers":[text],"answerKinds":["text"]}),
            ),
        }
    }
}

/// User-entered launch settings. Paths refer to the connected hub's machine.
#[derive(Clone, Debug, Default)]
pub struct NewSession {
    pub provider: String,
    pub cwd: String,
    pub label: String,
    pub model: String,
    pub message: String,
    pub context_window: Option<u64>,
    pub permission: Permission,
    /// Reasoning effort id; empty = the provider's default for the model.
    pub effort: String,
    pub resume_session_id: Option<String>,
}

impl NewSession {
    pub fn params(&self) -> Result<Value> {
        anyhow::ensure!(
            matches!(self.provider.as_str(), "claude" | "codex"),
            "Choose Claude or Codex"
        );
        let cwd = self.cwd.trim();
        anyhow::ensure!(
            absolute_directory(cwd),
            "Enter an absolute project directory on the hub's machine"
        );
        anyhow::ensure!(
            self.message.len() <= 65536,
            "Initial message must be at most 64 KiB"
        );
        let mode = self.permission.wire(&self.provider)?;
        let mut params = json!({"provider":self.provider,"cwd":cwd,"transport":"stream",
            "skipPermissions":self.permission == Permission::FullAccess,"permissionMode":mode});
        if let Some(id) = &self.resume_session_id {
            params["resumeSessionId"] = json!(id);
        }
        if let Some(window) = self.context_window {
            anyhow::ensure!(
                window > 0 && !self.model.trim().is_empty(),
                "Choose a model for the context window"
            );
            params["contextWindow"] = json!(window);
        }
        let effort = self.effort.trim();
        if !effort.is_empty() {
            anyhow::ensure!(
                crate::launch::valid_effort(effort),
                "Choose an effort level from the list"
            );
            params["effort"] = json!(effort);
        }
        for (key, value) in [
            ("label", &self.label),
            ("model", &self.model),
            ("message", &self.message),
        ] {
            if !value.trim().is_empty() {
                params[key] = json!(value.trim());
            }
        }
        Ok(params)
    }
}

#[derive(Clone, Debug)]
pub struct SpawnReceipt {
    pub number: u64,
    pub session: Option<String>,
    pub error: Option<String>,
    pub unsent_message: Option<String>,
}

pub enum Command {
    /// Consume a rendered UI intent; this does not acknowledge a remote tool.
    ConsumeUiRequest(u64),
    /// Bound to the pause the user actually saw, never a future stop episode.
    ResumePowerPause(u64),
    Select(String),
    OpenRecent(Box<Session>),
    Request(crate::features::Request),
    Act {
        session: String,
        action: Action,
    },
    Refresh,
    /// Widen the selected conversation's window by one page of older items.
    LoadOlder,
    /// Re-read account usage now rather than at the next minute.
    RefreshUsage,
    /// Show a provider-native subagent's conversation as the chat (read-only),
    /// or return to its parent with `None`.
    ViewChild(Option<ChildTarget>),
    Create(NewSession),
    LoadModels {
        key: CatalogKey,
        refresh: bool,
    },
}

/// A provider-native subagent of a session (Claude's Task/Agent children).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildTarget {
    pub parent: String,
    pub agent: String,
}

#[derive(Clone, Debug)]
pub struct Receipt {
    pub number: u64,
    pub session: String,
    pub action: Action,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PendingMessage {
    pub number: u64,
    pub session: String,
    pub text: String,
    pub queued: bool,
    pub accepted: bool,
    pub before: Option<crate::reading::Anchor>,
}

/// Recent completed recency updates, including failures, attributed to the
/// accepted launch's project and timestamp (not a later request in the slot).
#[derive(Clone, Debug)]
pub struct ProjectTouchReceipt {
    pub number: u64,
    pub path: String,
    pub at: i64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub ui_requests: Vec<crate::ui_requests::Request>,
    pub ui_request_warning: String,
    pub catalog: Catalog,
    pub requests: BTreeMap<&'static str, crate::features::RequestState>,
    /// Latest successful snapshot survives replacement/loading of request slots.
    pub project_registry: Option<Arc<Value>>,
    /// Last 128 completed touches; pending touches are never evicted or coalesced.
    pub project_touch_receipts: VecDeque<ProjectTouchReceipt>,
    pub connected: bool,
    pub power_paused: bool,
    pub power_pause_generation: u64,
    pub can_resume_power_pause: bool,
    pub sessions: Arc<Vec<Session>>,
    pub selected: Option<String>,
    pub transcript: Transcript,
    pub pending_messages: Vec<PendingMessage>,
    pub loading: bool,
    /// An "earlier messages" page is in flight.
    pub loading_older: bool,
    /// The last `usage.report` read; kept across failed refreshes.
    pub usage: Option<Arc<Value>>,
    /// When set, `transcript` is this subagent's conversation, not the
    /// selected session's; the parent stays `selected`.
    pub child: Option<ChildTarget>,
    pub sessions_loading: bool,
    pub busy: bool,
    pub notice: String,
    pub receipt: Option<Receipt>,
    pub creating: bool,
    pub spawn_receipt: Option<SpawnReceipt>,
}

#[derive(Clone)]
pub struct Controller {
    commands: tokio::sync::mpsc::Sender<Command>,
    pub views: tokio::sync::watch::Receiver<Arc<View>>,
}

impl Controller {
    pub(crate) fn channels() -> (
        Self,
        tokio::sync::mpsc::Receiver<Command>,
        tokio::sync::watch::Sender<Arc<View>>,
    ) {
        let (commands, incoming) = tokio::sync::mpsc::channel(32);
        let (updates, views) = tokio::sync::watch::channel(Arc::new(View::default()));
        (Self { commands, views }, incoming, updates)
    }

    /// UI fixtures use the same bounded commands and latest-state channel.
    #[cfg(feature = "ui-tests")]
    pub fn test_channels() -> (
        Self,
        tokio::sync::mpsc::Receiver<Command>,
        tokio::sync::watch::Sender<Arc<View>>,
    ) {
        Self::channels()
    }

    /// Attach within an existing runtime (protocol tests and headless harness).
    /// The desktop uses NativeHost to own its runtime off the UI thread.
    pub fn start(config: Config) -> Self {
        let (controller, incoming, updates) = Self::channels();
        tokio::spawn(async move {
            let (backend, events) = Backend::connect(config);
            Self::run(backend, events, incoming, updates).await;
        });
        controller
    }

    pub(crate) async fn run(
        backend: Backend,
        events: async_channel::Receiver<Event>,
        incoming: tokio::sync::mpsc::Receiver<Command>,
        updates: tokio::sync::watch::Sender<Arc<View>>,
    ) {
        Worker::new(backend).run(events, incoming, updates).await;
    }

    pub fn command(&self, command: Command) -> Result<()> {
        let command = match command {
            Command::Refresh => {
                let view = self.views.borrow();
                if view.power_paused {
                    Command::ResumePowerPause(view.power_pause_generation)
                } else {
                    Command::Refresh
                }
            }
            other => other,
        };
        self.commands
            .try_send(command)
            .map_err(|_| anyhow!("Client busy or stopped; action was not queued"))
    }
}

enum Completion {
    Request(u64, u64, crate::features::Request, Result<Value>),
    Models(u64, u64, Result<Vec<crate::launch::ModelChoice>>),
    Spawn(u64, NewSession, Result<Value>),
    Fleet(u64, Result<Value>),
    Usage(u64, Result<Value>),
    Child(u64, u64, Result<Value>),
    Conversation(u64, u64, Result<Value>),
    Action(u64, String, Action, Result<Value>),
}

struct Worker {
    backend: Backend,
    view: View,
    sessions: BTreeMap<String, Session>,
    jobs: FuturesUnordered<BoxFuture<'static, Completion>>,
    epoch: u64,
    selection: u64,
    fleet_pending: bool,
    fleet_overlay: BTreeMap<String, Value>,
    conversation_pending: bool,
    /// Items requested for the selected conversation (newest first).
    conversation_limit: usize,
    buffered: Vec<Delta>,
    buffered_bytes: usize,
    resync_after_read: bool,
    push_ready: bool,
    dirty: bool,
    fleet_dirty: bool,
    action_number: u64,
    catalog_number: u64,
    catalog_abort: Option<AbortHandle>,
    request_aborts: BTreeMap<&'static str, AbortHandle>,
    created_row: Option<Value>,
    last_fleet: Instant,
    last_conversation: Instant,
    usage_pending: bool,
    last_usage: Instant,
    child_pending: bool,
    last_child: Instant,
}

impl Worker {
    fn new(backend: Backend) -> Self {
        let can_resume_power_pause = backend.can_resume_power_pause();
        Self {
            backend,
            view: View {
                can_resume_power_pause,
                ..Default::default()
            },
            sessions: BTreeMap::new(),
            jobs: FuturesUnordered::new(),
            epoch: 0,
            selection: 0,
            fleet_pending: false,
            fleet_overlay: BTreeMap::new(),
            conversation_pending: false,
            conversation_limit: CONVERSATION_PAGE,
            buffered: Vec::new(),
            buffered_bytes: 0,
            resync_after_read: false,
            push_ready: false,
            dirty: false,
            fleet_dirty: false,
            action_number: 0,
            catalog_number: 0,
            catalog_abort: None,
            request_aborts: BTreeMap::new(),
            created_row: None,
            last_fleet: Instant::now(),
            last_conversation: Instant::now(),
            usage_pending: false,
            last_usage: Instant::now(),
            child_pending: false,
            last_child: Instant::now(),
        }
    }

    async fn run(
        mut self,
        events: async_channel::Receiver<Event>,
        mut commands: tokio::sync::mpsc::Receiver<Command>,
        updates: tokio::sync::watch::Sender<Arc<View>>,
    ) {
        let mut frame = tokio::time::interval(FRAME_INTERVAL);
        frame.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut maintenance = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    None => break,
                    Some(Command::Select(id)) if self.sessions.contains_key(&id) => self.select(Some(id)).await,
                    Some(Command::OpenRecent(session)) => {
                        let id = session.id.clone();
                        self.sessions.entry(id.clone()).or_insert(*session);
                        self.fleet_dirty = true;
                        self.select(Some(id)).await;
                    }
                    Some(Command::ConsumeUiRequest(number)) => {
                        self.view.ui_requests.retain(|r|r.number!=number);
                        self.dirty=true;
                    }
                    Some(Command::Request(request)) => self.request(request),
                    Some(Command::Act { session, action }) => self.act(session, action),
                    Some(Command::Create(request)) => self.create(request),
                    Some(Command::LoadModels { key, refresh }) => self.load_models(key, refresh),
                    Some(Command::ResumePowerPause(generation)) => {
                        if self.view.power_paused && self.view.power_pause_generation==generation {
                            match self.backend.resume_power_pause() {
                                Ok(())=>{self.view.power_paused=false;self.view.notice="Reconnecting at your request…".into();},
                                Err(error)=>self.view.notice=error.to_string(),
                            }
                            self.dirty=true;
                        }
                    }
                    Some(Command::RefreshUsage) => self.fetch_usage(),
                    Some(Command::ViewChild(target)) => self.view_child(target).await,
                    Some(Command::LoadOlder) => {
                        if self.view.transcript.has_older && !self.conversation_pending {
                            self.conversation_limit += CONVERSATION_PAGE;
                            self.view.loading_older = true;
                            self.fetch_conversation();
                            self.dirty = true;
                        }
                    }
                    Some(Command::Refresh) => {
                        self.view.loading = self.view.connected && self.view.selected.is_some();
                        self.fetch_fleet();
                        if self.view.child.is_some() { self.fetch_child(); } else { self.fetch_conversation(); }
                        self.dirty=true;
                    }
                    _ => {}
                },
                event = events.recv() => match event {
                    Err(_) => {
                        if self.view.connected {self.disconnected("Backend event stream closed".into(),false);}
                        if self.fleet_dirty {self.view.sessions=Arc::new(self.sessions.values().cloned().collect());}
                        updates.send_replace(Arc::new(self.view.clone()));
                        break;
                    },
                    Ok(event) => self.event(event).await,
                },
                Some(result) = self.jobs.next(), if !self.jobs.is_empty() => self.complete(result).await,
                _ = frame.tick(), if self.dirty => {
                    if self.fleet_dirty {
                        self.view.sessions = Arc::new(self.sessions.values().cloned().collect());
                        self.fleet_dirty = false;
                    }
                    self.reconcile_messages();
                    let view = Arc::new(self.view.clone());
                    // Replaces the latest state even when no window is open.
                    updates.send_replace(view);
                    self.dirty = false;
                }
                _ = maintenance.tick() => {
                    if self.view.connected {
                        if self.last_fleet.elapsed() >= Duration::from_secs(30) { self.fetch_fleet(); }
                        // The hub's report is valid for 60s; account windows move slowly.
                        if self.last_usage.elapsed() >= Duration::from_secs(60) { self.fetch_usage(); }
                        // Ready suppresses fast polling. A slow reconciliation also
                        // repairs a provider restart behind an otherwise healthy hub.
                        let pace = if self.push_ready { Duration::from_secs(30) } else { Duration::from_secs(1) };
                        if self.view.child.is_some() {
                            // Subagent transcripts have no push feed: poll while it runs.
                            if self.child_running() && self.last_child.elapsed() >= Duration::from_secs(2) { self.fetch_child(); }
                        } else if self.last_conversation.elapsed() >= pace { self.fetch_conversation(); }
                    }
                }
            }
        }
        // Dropping jobs and bus closes pending calls and releases subscriptions.
    }

    fn reconcile_messages(&mut self) {
        if self.view.loading {
            return;
        }
        let rows = &self.view.transcript.rows;
        let mut consumed_through: Option<usize> = None;
        self.view.pending_messages.retain_mut(|pending| {
            if self.view.selected.as_ref() != Some(&pending.session) {
                return true;
            }
            let start = match &pending.before {
                Some(anchor) => match anchor.locate(rows) {
                    Some(ix) => ix + 1,
                    None => return true,
                },
                None => 0,
            };
            let start = start.max(consumed_through.map_or(0, |ix| ix + 1));
            if let Some((ix, _)) = rows
                .iter()
                .enumerate()
                .skip(start)
                .find(|(_, r)| r.role == "You" && r.text == pending.text)
            {
                consumed_through = Some(ix);
                false
            } else {
                // Retain the consumed boundary across frames. Otherwise a second
                // identical queued send could reuse the first send's echo later.
                if let Some(ix) = consumed_through {
                    pending.before = crate::reading::Anchor::at(rows, ix);
                }
                true
            }
        });
    }

    fn request(&mut self, request: crate::features::Request) {
        if let crate::features::Request::SubagentHistory { session, agent } = &request
            && (self.view.selected.as_ref() != Some(session)
                || !self.sessions.get(session).is_some_and(|parent| {
                    parent.subagents.as_array().is_some_and(|children| {
                        children
                            .iter()
                            .any(|child| child["id"].as_str() == Some(agent.as_str()))
                    })
                }))
        {
            return;
        }
        let key = request.key();
        // A write already on its way cannot be recalled: superseding it would
        // report "superseded" for a change the hub may still apply.
        if matches!(key, "upload" | "project-save")
            && self.view.requests.get(key).is_some_and(|s| s.loading)
        {
            return;
        }
        self.action_number += 1;
        let number = self.action_number;
        // Each accepted touch keeps its own job. Jobs queue on the shared
        // per-hub FIFO transaction lock; never cancel an acknowledged launch's
        // recency update merely because another launch used the same slot.
        if key != "project-touch"
            && let Some(abort) = self.request_aborts.remove(key)
        {
            abort.abort();
        }
        let (abort, registration) = AbortHandle::new_pair();
        if key != "project-touch" {
            self.request_aborts.insert(key, abort);
        }
        self.view.requests.insert(
            key,
            crate::features::RequestState {
                number,
                request: request.clone(),
                loading: true,
                value: Arc::new(Value::Null),
                error: None,
            },
        );
        let backend = self.backend.clone();
        let epoch = self.epoch;
        self.jobs.push(Box::pin(async move {
            let result = Abortable::new(request.run(&backend), registration)
                .await
                .unwrap_or_else(|_| Err(anyhow!("Request superseded")));
            Completion::Request(epoch, number, request, result)
        }));
        self.dirty = true;
    }

    fn load_models(&mut self, key: CatalogKey, refresh: bool) {
        if !matches!(key.provider.as_str(), "claude" | "codex") {
            return;
        }
        if self.view.catalog.key == key
            && (self.view.catalog.loading
                || (!refresh
                    && self.view.catalog.error.is_none()
                    && !self.view.catalog.models.is_empty()))
        {
            return;
        }
        self.catalog_number += 1;
        if let Some(abort) = self.catalog_abort.take() {
            abort.abort();
        }
        if self.view.catalog.key != key {
            self.view.catalog = Catalog {
                key: key.clone(),
                ..Default::default()
            };
        }
        self.view.catalog.error = None;
        self.dirty = true;
        if !self.view.connected
            || (key.provider != "claude" && !key.cwd.is_empty() && !absolute_directory(&key.cwd))
        {
            self.view.catalog.loading = false;
            self.view.catalog.error = Some(
                if !self.view.connected {
                    "Connect to the hub to load models."
                } else {
                    "Enter an absolute project directory to load models."
                }
                .into(),
            );
            return;
        }
        self.view.catalog.loading = true;
        let backend = self.backend.clone();
        let epoch = self.epoch;
        let number = self.catalog_number;
        let (abort, registration) = AbortHandle::new_pair();
        self.catalog_abort = Some(abort);
        self.jobs.push(Box::pin(async move {
            let result = Abortable::new(backend.models(&key), registration)
                .await
                .unwrap_or_else(|_| Err(anyhow!("Model query superseded")))
                .and_then(|value| parse_models(&key.provider, value));
            Completion::Models(epoch, number, result)
        }));
    }

    fn fetch_fleet(&mut self) {
        if !self.view.connected || self.fleet_pending {
            return;
        }
        self.fleet_pending = true;
        self.view.sessions_loading = true;
        self.dirty = true;
        self.fleet_overlay.clear();
        self.last_fleet = Instant::now();
        let backend = self.backend.clone();
        let epoch = self.epoch;
        self.jobs.push(Box::pin(async move {
            Completion::Fleet(epoch, backend.snapshots().await)
        }));
    }

    fn fetch_usage(&mut self) {
        if !self.view.connected || self.usage_pending {
            return;
        }
        self.usage_pending = true;
        self.last_usage = Instant::now();
        let backend = self.backend.clone();
        let epoch = self.epoch;
        self.jobs.push(Box::pin(async move {
            Completion::Usage(epoch, backend.usage_report().await)
        }));
    }

    fn child_running(&self) -> bool {
        let Some(target) = &self.view.child else {
            return false;
        };
        self.sessions
            .get(&target.parent)
            .and_then(|s| s.subagents.as_array())
            .into_iter()
            .flatten()
            .find(|c| c["id"].as_str() == Some(target.agent.as_str()))
            .is_some_and(|c| {
                matches!(
                    c["status"].as_str(),
                    Some(
                        "running"
                            | "responding"
                            | "streaming"
                            | "working"
                            | "thinking"
                            | "background"
                    )
                )
            })
    }

    fn fetch_child(&mut self) {
        let Some(target) = self.view.child.clone() else {
            return;
        };
        if !self.view.connected || self.child_pending {
            return;
        }
        self.child_pending = true;
        self.last_child = Instant::now();
        let backend = self.backend.clone();
        let (epoch, selection) = (self.epoch, self.selection);
        self.jobs.push(Box::pin(async move {
            let result = backend
                .call(
                    "sessions.subagentConversation",
                    json!({"sessionId":target.parent,"agentId":target.agent}),
                )
                .await;
            Completion::Child(epoch, selection, result)
        }));
    }

    /// Enter or leave a subagent's conversation. Entering selects the parent
    /// first; the selection fence then drops any in-flight parent read, and
    /// parent deltas are ignored until the reader returns.
    async fn view_child(&mut self, target: Option<ChildTarget>) {
        if target == self.view.child {
            return;
        }
        if let Some(target) = &target
            && self.view.selected.as_ref() != Some(&target.parent)
        {
            if !self.sessions.contains_key(&target.parent) {
                return;
            }
            self.select(Some(target.parent.clone())).await;
        }
        let entering = target.is_some();
        self.selection += 1;
        self.view.child = target;
        self.view.transcript = Transcript::default();
        self.view.loading = true;
        self.view.loading_older = false;
        self.conversation_pending = false;
        self.child_pending = false;
        self.buffered.clear();
        self.buffered_bytes = 0;
        if entering {
            self.fetch_child();
        } else {
            self.conversation_limit = CONVERSATION_PAGE;
            self.fetch_conversation();
        }
        self.dirty = true;
    }

    fn fetch_conversation(&mut self) {
        if !self.view.connected || self.conversation_pending || self.view.child.is_some() {
            return;
        }
        let Some(id) = self.view.selected.clone() else {
            return;
        };
        self.conversation_pending = true;
        self.buffered.clear();
        self.buffered_bytes = 0;
        self.resync_after_read = false;
        self.last_conversation = Instant::now();
        let backend = self.backend.clone();
        let epoch = self.epoch;
        let selection = self.selection;
        // Reconciliation and gap repairs re-read the same window, never the
        // whole retained log.
        let limit = Some(self.conversation_limit);
        self.jobs.push(Box::pin(async move {
            Completion::Conversation(epoch, selection, backend.conversation(&id, limit).await)
        }));
    }

    async fn select(&mut self, id: Option<String>) {
        self.selection += 1;
        self.view.selected = id;
        self.view.child = None;
        self.child_pending = false;
        for key in [
            "history",
            "subagent-history",
            "changes",
            "diff",
            "card-diff",
            "file-preview",
        ] {
            self.view.requests.remove(key);
            if let Some(abort) = self.request_aborts.remove(key) {
                abort.abort();
            }
        }
        self.view.transcript = Transcript::default();
        self.view.loading = self.view.selected.is_some();
        self.view.loading_older = false;
        self.conversation_limit = CONVERSATION_PAGE;
        self.conversation_pending = false;
        self.push_ready = false;
        self.buffered.clear();
        let mut topics = BTreeSet::from(["agent.snapshot".into()]);
        topics.extend(crate::ui_requests::TOPICS.iter().map(|s| s.to_string()));
        if let Some(id) = &self.view.selected {
            topics.insert(format!("agent.conversation.{id}"));
        }
        let _ = self.backend.topics(topics).await;
        self.fetch_conversation();
        self.dirty = true;
    }

    fn upsert(&mut self, data: &Value) {
        let Some(id) = Session::id_of(data) else {
            return;
        };
        if !self.sessions.contains_key(id) && self.sessions.len() >= MAX_SESSIONS {
            return;
        }
        self.sessions.entry(id.into()).or_default().merge(data);
        self.fleet_dirty = true;
    }

    fn disconnected(&mut self, reason: String, power_paused: bool) {
        self.epoch += 1;
        self.view.connected = false;
        self.view.ui_requests.clear();
        for state in self.view.requests.values_mut() {
            if state.loading {
                state.loading = false;
                state.error = Some("Connection lost. Check the result before retrying.".into());
            }
        }
        self.view.catalog.loading = false;
        self.view.catalog.error = Some("Hub disconnected. Reconnect to load models.".into());
        self.view.loading = false;
        self.view.sessions_loading = false;
        self.view.power_paused = power_paused;
        if power_paused {
            self.view.power_pause_generation = self.epoch;
        }
        self.view.notice = if power_paused {
            if self.view.can_resume_power_pause {
                "Server requested a reconnect pause. Reconnect when you want to inspect or wake it."
                    .into()
            } else {
                "Local hub connection is paused. Restart it through its owning host when ready."
                    .into()
            }
        } else if self.view.can_resume_power_pause {
            format!("{reason}. Reconnecting…")
        } else {
            reason
        };
        self.push_ready = false;
    }

    async fn event(&mut self, event: Event) {
        match event {
            Event::Connected => {
                self.epoch += 1;
                self.view.connected = true;
                self.view.power_paused = false;
                self.view.notice.clear();
                self.fleet_pending = false;
                self.conversation_pending = false;
                self.usage_pending = false;
                self.fetch_fleet();
                self.fetch_usage();
                let child = self.view.child.clone();
                self.select(self.view.selected.clone()).await;
                if child.is_some() {
                    self.view_child(child).await;
                }
            }
            Event::Disconnected(reason) => self.disconnected(reason, false),
            Event::PowerPaused => self.disconnected(String::new(), true),
            Event::Data { topic, data, hub } => {
                // This first client operates on the connected hub only. Never
                // interpret a federated session as locally owned.
                if hub.is_some()
                    || data
                        .get("hub")
                        .and_then(Value::as_str)
                        .is_some_and(|h| !h.is_empty())
                {
                    return;
                }
                if crate::ui_requests::TOPICS.contains(&topic.as_str()) {
                    match crate::ui_requests::parse(&topic, &data) {
                        Ok(Some((intent, payload)))
                            if self.view.ui_requests.len() < crate::ui_requests::MAX_PENDING =>
                        {
                            self.action_number += 1;
                            self.view.ui_requests.push(crate::ui_requests::Request {
                                number: self.action_number,
                                intent,
                                payload,
                            });
                        }
                        Ok(Some(_)) => {
                            self.view.ui_request_warning =
                                "UI request queue is full; the new request was not applied.".into()
                        }
                        Err(error) => self.view.ui_request_warning = error.to_string(),
                        Ok(None) => (),
                    }
                } else if topic == "agent.snapshot" {
                    if self.fleet_pending
                        && let Some(id) = Session::id_of(&data)
                        && self.fleet_overlay.len() < MAX_SESSIONS
                    {
                        // Store the projection rather than retaining rich transcripts.
                        let mut merged = self.fleet_overlay.remove(id).unwrap_or(json!({}));
                        for key in [
                            "sessionId",
                            "session_id",
                            "label",
                            "customName",
                            "cwd",
                            "mode",
                            "ambientState",
                            "status",
                            "transport",
                            "provider",
                            "parentSessionId",
                            "parent_session_id",
                            "model",
                            "requestedSelection",
                            "settings",
                            "pending",
                            "pendingApproval",
                            "pendingQuestions",
                        ] {
                            if let Some(v) = data.get(key) {
                                merged[key] = v.clone();
                            }
                        }
                        self.fleet_overlay.insert(id.into(), merged);
                    }
                    self.upsert(&data);
                } else if self.view.child.is_none()
                    && self
                        .view
                        .selected
                        .as_ref()
                        .is_some_and(|id| topic == format!("agent.conversation.{id}"))
                {
                    let size = data.to_string().len();
                    if let Ok(delta) = serde_json::from_value::<Delta>(data) {
                        if delta.ready {
                            self.push_ready = true;
                            // A subscribe ack alone does not establish the
                            // provider's demand/SSE path. Reseed after ready so
                            // updates between the first read and readiness are
                            // observed even when there is no subsequent delta.
                            if self.conversation_pending {
                                self.resync_after_read = true;
                            } else {
                                self.fetch_conversation();
                            }
                        } else if self.conversation_pending {
                            if delta.reset
                                || self.buffered.len() >= 64
                                || self.buffered_bytes + size > 2 * 1024 * 1024
                            {
                                self.buffered.clear();
                                self.resync_after_read = true;
                            } else if !self.resync_after_read {
                                self.buffered_bytes += size;
                                self.buffered.push(delta);
                            }
                        } else {
                            self.push_ready = true;
                            // RPC completions and events use different queues.
                            // Even a reset received before a reply on the wire
                            // may be processed after it here. Always read after
                            // resets, including when no read appears pending.
                            if delta.reset {
                                self.view.loading = true;
                                self.fetch_conversation();
                            } else if self.view.transcript.delta(delta, self.streaming())
                                == Fold::Gap
                            {
                                self.fetch_conversation();
                            }
                        }
                    }
                }
            }
        }
        self.dirty = true;
    }

    fn streaming(&self) -> bool {
        self.view
            .selected
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(|s| s.transport == "stream")
    }

    fn act(&mut self, session: String, action: Action) {
        if self.view.busy {
            return;
        }
        self.action_number += 1;
        let number = self.action_number;
        let valid = self.view.connected
            && self.view.selected.as_ref() == Some(&session)
            && self.sessions.get(&session).is_some_and(|s| {
                !s.stopped()
                    && match &action {
                        Action::Approve(_) => s.approval.is_some(),
                        Action::Answer(text) => {
                            s.questions.is_some() && !text.trim().is_empty() && text.len() <= 65536
                        }
                        Action::Send(text) => {
                            !self.view.loading && !text.trim().is_empty() && text.len() <= 65536
                        }
                        Action::Stop | Action::Terminate => true,
                        Action::Answers(answers) => {
                            s.questions.is_some()
                                && !answers.is_empty()
                                && answers.len() <= 20
                                && answers
                                    .iter()
                                    .all(|s| !s.trim().is_empty() && s.len() <= 65536)
                        }
                        Action::SetModel { model, .. } => {
                            !model.trim().is_empty() && model.len() < 256
                        }
                    }
            });
        if !valid {
            self.view.receipt = Some(Receipt {
                number,
                session,
                action,
                error: Some(
                    "Action unavailable for this session or connection; nothing was sent".into(),
                ),
            });
        } else {
            if let Action::Send(text) = &action {
                if self.view.pending_messages.len() >= 32 {
                    self.view.receipt = Some(Receipt { number, session, action, error: Some("Too many unacknowledged messages; wait for delivery before sending more".into()) });
                    self.dirty = true;
                    return;
                }
                let rows = &self.view.transcript.rows;
                let before = rows
                    .iter()
                    .rposition(|r| r.role == "You")
                    .and_then(|ix| crate::reading::Anchor::at(rows, ix));
                let queued = self.sessions.get(&session).is_some_and(|s| {
                    !matches!(s.state.as_str(), "input" | "idle" | "done" | "background")
                }) || self
                    .view
                    .pending_messages
                    .iter()
                    .any(|p| p.session == session);
                self.view.pending_messages.push(PendingMessage {
                    number,
                    session: session.clone(),
                    text: text.clone(),
                    queued,
                    accepted: false,
                    before,
                });
            }
            self.view.busy = true;
            let backend = self.backend.clone();
            let stream = self
                .sessions
                .get(&session)
                .is_some_and(|s| s.transport == "stream");
            self.jobs.push(Box::pin(async move {
                let result = backend.action(&session, &action, stream).await;
                Completion::Action(number, session, action, result)
            }));
        }
        self.dirty = true;
    }

    fn create(&mut self, request: NewSession) {
        if self.view.creating {
            return;
        }
        self.action_number += 1;
        let number = self.action_number;
        let params = request.params().and_then(|params| {
            anyhow::ensure!(
                self.view.connected,
                "Hub disconnected; session was not created"
            );
            Ok(params)
        });
        match params {
            Err(error) => {
                self.view.spawn_receipt = Some(SpawnReceipt {
                    number,
                    session: None,
                    error: Some(error.to_string()),
                    unsent_message: None,
                });
            }
            Ok(params) => {
                self.view.creating = true;
                let backend = self.backend.clone();
                self.jobs.push(Box::pin(async move {
                    let result = backend.spawn(params).await;
                    Completion::Spawn(number, request, result)
                }));
            }
        }
        self.dirty = true;
    }

    async fn complete(&mut self, completion: Completion) {
        match completion {
            Completion::Request(epoch, number, request, result) => {
                let key = request.key();
                if epoch != self.epoch
                    || (key != "project-touch"
                        && self
                            .view
                            .requests
                            .get(key)
                            .is_none_or(|s| s.number != number))
                {
                    return;
                }
                let (value, error) = match result {
                    Ok(v) => (v, None),
                    Err(e) => (Value::Null, Some(e.to_string())),
                };
                let value = Arc::new(value);
                if matches!(key, "projects" | "project-save" | "project-touch")
                    && error.is_none()
                    && value["revision"].as_u64().unwrap_or(0)
                        > self
                            .view
                            .project_registry
                            .as_ref()
                            .and_then(|v| v["revision"].as_u64())
                            .unwrap_or(0)
                {
                    self.view.project_registry = Some(value.clone());
                }
                if let crate::features::Request::TouchProject { path, at } = &request {
                    self.view
                        .project_touch_receipts
                        .push_back(ProjectTouchReceipt {
                            number,
                            path: path.clone(),
                            at: *at,
                            error: error.clone(),
                        });
                    if self.view.project_touch_receipts.len() > 128 {
                        self.view.project_touch_receipts.pop_front();
                    }
                }
                self.view.requests.insert(
                    key,
                    crate::features::RequestState {
                        number,
                        request,
                        loading: false,
                        value,
                        error,
                    },
                );
            }
            Completion::Models(epoch, number, result) => {
                if epoch != self.epoch || number != self.catalog_number {
                    return;
                }
                self.view.catalog.loading = false;
                match result {
                    Ok(models) if !models.is_empty() => {
                        self.view.catalog.models = models;
                        self.view.catalog.error = None;
                    }
                    Ok(_) => self.view.catalog.error = Some("No models returned. Use the provider default, enter a custom model, or retry.".into()),
                    Err(error) => self.view.catalog.error = Some(format!("Could not load models: {error}")),
                }
            }
            Completion::Spawn(number, request, result) => {
                self.view.creating = false;
                let result = result.and_then(|value| {
                    anyhow::ensure!(value["sessionId"].as_str().is_some_and(|id| !id.is_empty()),
                        "Spawn returned no session ID; outcome unknown. Refresh sessions before retrying");
                    Ok(value)
                });
                match result {
                    Ok(value) => {
                        let id = value["sessionId"].as_str().unwrap().to_owned();
                        let unsent_message = (!request.message.trim().is_empty()
                            && value["messageQueued"] != true)
                            .then(|| request.message.clone());
                        let row = json!({"sessionId":id,"cwd":request.cwd.trim(),"label":request.label.trim(),"transport":"stream","mode":"unknown","provider":request.provider,"model":request.model});
                        // Preserve the acknowledged identity across an older fleet read.
                        if !self.sessions.contains_key(&id) {
                            self.upsert(&row);
                        }
                        self.created_row = Some(row);
                        self.select(Some(id.clone())).await;
                        self.view.notice = if unsent_message.is_some() {
                            "Session created. Initial message delivery was not confirmed; check the conversation before sending the retained draft.".into()
                        } else {
                            "Session created".into()
                        };
                        self.view.spawn_receipt = Some(SpawnReceipt {
                            number,
                            session: Some(id),
                            error: None,
                            unsent_message,
                        });
                        self.fetch_fleet();
                    }
                    Err(error) => {
                        self.view.spawn_receipt = Some(SpawnReceipt {
                            number,
                            session: None,
                            error: Some(error.to_string()),
                            unsent_message: None,
                        });
                    }
                }
            }
            Completion::Child(epoch, selection, result)
                if epoch == self.epoch && selection == self.selection =>
            {
                self.child_pending = false;
                self.view.loading = false;
                let Some(target) = self.view.child.clone() else {
                    return;
                };
                let snapshot = result.and_then(|value| {
                    anyhow::ensure!(!value.is_null(), "Subagent transcript is not available yet");
                    anyhow::ensure!(
                        value["session_id"]
                            .as_str()
                            .is_none_or(|id| id == target.parent)
                            && value["agent_id"]
                                .as_str()
                                .is_none_or(|id| id == target.agent),
                        "Subagent transcript belongs to another session"
                    );
                    Ok(serde_json::from_value::<ConversationSnapshot>(value)?)
                });
                match snapshot {
                    Ok(snapshot) => {
                        // Same fold and identity-stable keys as a parent reseed,
                        // so polling keeps the reader's place.
                        self.view.transcript.snapshot_for_transport(snapshot, false);
                        if self.view.notice.starts_with("Conversation unavailable:") {
                            self.view.notice.clear();
                        }
                    }
                    Err(e) => self.view.notice = format!("Conversation unavailable: {e}"),
                }
                self.dirty = true;
            }
            Completion::Usage(epoch, result) if epoch == self.epoch => {
                self.usage_pending = false;
                // Older hubs lack usage.report; a failed refresh keeps the last
                // reading, whose rolled-over windows drop out on their own.
                if let Ok(report) = result {
                    self.view.usage = Some(Arc::new(report));
                    self.dirty = true;
                }
            }
            Completion::Fleet(epoch, result) if epoch == self.epoch => {
                self.fleet_pending = false;
                self.view.sessions_loading = false;
                match result {
                    Ok(Value::Array(rows)) => {
                        if self.view.notice.starts_with("Sessions unavailable:") {
                            self.view.notice.clear();
                        }
                        let retained = self
                            .view
                            .selected
                            .as_ref()
                            .and_then(|id| self.sessions.get(id))
                            .filter(|s| s.stopped())
                            .cloned();
                        self.sessions.clear();
                        if let Some(s) = retained {
                            self.sessions.insert(s.id.clone(), s);
                        }
                        for row in rows.into_iter().take(MAX_SESSIONS) {
                            if row
                                .get("hub")
                                .and_then(Value::as_str)
                                .is_none_or(str::is_empty)
                            {
                                self.upsert(&row);
                            }
                        }
                        for (_, row) in std::mem::take(&mut self.fleet_overlay) {
                            self.upsert(&row);
                        }
                        if let Some(row) = self.created_row.take() {
                            let id = row["sessionId"].as_str().unwrap();
                            if !self.sessions.contains_key(id) {
                                self.upsert(&row);
                                self.created_row = Some(row);
                            }
                        }
                        self.fleet_dirty = true;
                        if self
                            .view
                            .selected
                            .as_ref()
                            .is_none_or(|id| !self.sessions.contains_key(id))
                        {
                            self.select(self.sessions.keys().next().cloned()).await;
                        }
                    }
                    Ok(_) => {
                        self.view.notice =
                            "Sessions unavailable: hub returned an invalid session list".into()
                    }
                    Err(e) => self.view.notice = format!("Sessions unavailable: {e}"),
                }
            }
            Completion::Conversation(epoch, selection, result)
                if epoch == self.epoch && selection == self.selection =>
            {
                self.conversation_pending = false;
                self.view.loading = false;
                self.view.loading_older = false;
                let window_first_seq = result
                    .as_ref()
                    .ok()
                    .and_then(|v| v["window_first_seq"].as_u64());
                match result.and_then(|v| {
                    serde_json::from_value::<ConversationSnapshot>(v).map_err(Into::into)
                }) {
                    Ok(snapshot) => {
                        // Sequence counters restart on reset. A buffered reset
                        // and an in-flight snapshot cannot be ordered by seq:
                        // either may describe the old generation. Read again
                        // after observing the reset rather than overwrite newer
                        // state with the reset's earlier retained window.
                        if self.resync_after_read {
                            self.view.loading = true;
                            self.fetch_conversation();
                            self.dirty = true;
                            return;
                        }
                        let streaming = self.streaming();
                        self.view
                            .transcript
                            .snapshot_page(snapshot, window_first_seq, streaming);
                        if self.view.notice.starts_with("Conversation unavailable:") {
                            self.view.notice.clear();
                        }
                        let mut gap = false;
                        for delta in self.buffered.drain(..) {
                            if self.view.transcript.delta(delta, streaming) == Fold::Gap {
                                gap = true;
                                break;
                            }
                        }
                        if gap {
                            self.fetch_conversation();
                        }
                    }
                    Err(e) => {
                        self.push_ready = false;
                        self.view.notice = format!("Conversation unavailable: {e}");
                    }
                }
            }
            Completion::Action(number, session, action, result) => {
                self.view.busy = false;
                let queued = result
                    .as_ref()
                    .is_ok_and(|v| v["queued"] == true || v["disposition"] == "queued");
                let error = result
                    .and_then(|v| {
                        anyhow::ensure!(
                            v["ok"] != false,
                            "{}",
                            v["error"].as_str().unwrap_or("Request refused")
                        );
                        Ok(v)
                    })
                    .err()
                    .map(|e| e.to_string());
                if error.is_some() {
                    self.view.pending_messages.retain(|p| p.number != number);
                } else if let Some(pending) = self
                    .view
                    .pending_messages
                    .iter_mut()
                    .find(|p| p.number == number)
                {
                    pending.accepted = true;
                    pending.queued |= queued;
                }
                self.view.notice = error.clone().unwrap_or_else(|| {
                    if queued && !matches!(&action, Action::Send(_)) {
                        "Change queued; the provider will apply it when ready".into()
                    } else {
                        String::new()
                    }
                });
                self.view.receipt = Some(Receipt {
                    number,
                    session,
                    action,
                    error,
                });
                self.fetch_fleet();
                self.fetch_conversation();
            }
            _ => return, // result belongs to an older connection or selection
        }
        self.dirty = true;
    }
}
