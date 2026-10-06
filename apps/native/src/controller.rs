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
        /// Then also switch effort (`claude.setEffort`, after the model call
        /// succeeds); `None` leaves the session's effort alone.
        effort: Option<String>,
    },
    /// Live reasoning-effort switch: Claude's `/effort` command or Codex's
    /// thread settings, chosen by the hub per provider.
    SetEffort(String),
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
                ..
            } => (
                "claude.setModel",
                json!({"sessionId":id,"model":model,"modelIdentity":model,"contextWindow":context_window}),
            ),
            Self::SetEffort(effort) => {
                ("claude.setEffort", json!({"sessionId":id,"effort":effort}))
            }
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
        // No name given: ask the owning hub to name it after its first
        // exchange (`agents.autoTitle`). A typed label is never replaced.
        if self.label.trim().is_empty() {
            params["autoTitle"] = json!(true);
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
    /// Continue a session with the other provider; see [`crate::handoff`].
    Handoff(crate::handoff::Request),
    LoadModels {
        key: CatalogKey,
        refresh: bool,
    },
    /// An agent's interactive shell; see [`crate::terminal`].
    Terminal(crate::terminal::Command),
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

/// A completed archive/restore, including failures, so the UI can settle the
/// exact change it optimistically showed (not a later one for the session).
#[derive(Clone, Debug)]
pub struct ArchiveReceipt {
    pub number: u64,
    pub session: String,
    pub archived: bool,
    pub error: Option<String>,
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
    /// The hub's shared archived-session document (newest version seen), or
    /// `None` while unknown or when this hub predates shared archives.
    pub session_archive: Option<Arc<Value>>,
    /// Last 128 completed archive/restore requests.
    pub archive_receipts: VecDeque<ArchiveReceipt>,
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
    /// Why the latest `usage.report` read failed; cleared by the next success.
    pub usage_error: Option<String>,
    /// When set, `transcript` is this subagent's conversation, not the
    /// selected session's; the parent stays `selected`.
    pub child: Option<ChildTarget>,
    pub sessions_loading: bool,
    pub busy: bool,
    pub notice: String,
    pub receipt: Option<Receipt>,
    pub creating: bool,
    pub spawn_receipt: Option<SpawnReceipt>,
    /// The handoff in flight (at most one per connection).
    pub handoff: Option<crate::handoff::Progress>,
    /// The latest finished handoff, including refusals and failures.
    pub handoff_receipt: Option<crate::handoff::Receipt>,
    /// Each agent's shell, by agent session id. Shell sessions themselves
    /// never appear in `sessions`.
    pub terminals: BTreeMap<String, crate::terminal::Terminal>,
    /// Shell output waiting for the window (shared, not snapshotted).
    pub terminal_feed: Arc<crate::terminal::Feed>,
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
    Spawn(u64, NewSession, Option<HandoffSpawn>, Result<Value>),
    HandoffBrief(u64, crate::handoff::Request, Result<Value>),
    Fleet(u64, Result<Value>),
    Usage(u64, Result<Value>),
    Child(u64, u64, Result<Value>),
    Conversation(u64, u64, Result<Value>),
    Action(u64, String, Action, Result<Value>),
    /// (epoch, agent, shell generation, step)
    Terminal(u64, String, u64, ShellStep),
}

/// A successor launch's handoff: whose work it continues and its brief.
struct HandoffSpawn {
    source: String,
    brief: crate::handoff::Written,
}

enum ShellStep {
    Created(Result<Value>),
    Attached(Result<Value>),
    Input(Result<Value>),
    Resized,
    Keepalive(Result<Value>),
}

/// Worker-side bookkeeping for one agent's shell; the window's projection is
/// `View::terminals`.
struct Shell {
    id: Option<String>,
    cwd: String,
    /// The window shows this terminal: keep its output streaming.
    wanted: bool,
    attached: bool,
    /// Bumped when the shell is replaced; older completions are ignored.
    generation: u64,
    input: Vec<u8>,
    input_busy: bool,
    size: (u16, u16),
    resize_pending: bool,
    resize_busy: bool,
    last_keepalive: Instant,
}

/// A transcript left by a parent/child switch in this connection epoch. Going
/// back shows it at once while the usual fenced read reconciles it in place.
struct Held {
    epoch: u64,
    transcript: Transcript,
    /// The parent's conversation window, so its reconciling read keeps any
    /// pages the user had loaded.
    limit: usize,
}

/// Children held for quick return (newest last), and their retained text.
const MAX_HELD_CHILDREN: usize = 8;
const MAX_HELD_BYTES: usize = 16 * 1024 * 1024;

/// Keyboard input is batched per call; this much may queue behind a slow hub.
const MAX_TERMINAL_INPUT: usize = 1024 * 1024;
const TERMINAL_KEEPALIVE: Duration = Duration::from_secs(8);

struct Worker {
    backend: Backend,
    view: View,
    sessions: BTreeMap<String, Session>,
    jobs: FuturesUnordered<BoxFuture<'static, Completion>>,
    epoch: u64,
    archive_resync: bool,
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
    held_parent: Option<(String, Held)>,
    held_children: VecDeque<(ChildTarget, Held)>,
    shells: BTreeMap<String, Shell>,
    /// Shell ids replaced or ended in this run; their rows stay hidden.
    retired_shells: BTreeSet<String>,
    shell_generation: u64,
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
            archive_resync: true,
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
            held_parent: None,
            held_children: VecDeque::new(),
            shells: BTreeMap::new(),
            retired_shells: BTreeSet::new(),
            shell_generation: 0,
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
                    Some(Command::Handoff(request)) => self.handoff(request),
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
                    Some(Command::Terminal(command)) => self.terminal(command).await,
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
                        if self.fleet_dirty {self.view.sessions=self.session_list();}
                        updates.send_replace(Arc::new(self.view.clone()));
                        break;
                    },
                    Ok(event) => self.event(event).await,
                },
                Some(result) = self.jobs.next(), if !self.jobs.is_empty() => self.complete(result).await,
                _ = frame.tick(), if self.dirty => {
                    if self.fleet_dirty {
                        self.view.sessions = self.session_list();
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
                        self.keep_terminals_alive();
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
        if matches!(key, "upload" | "project-save" | "file-save")
            && self.view.requests.get(key).is_some_and(|s| s.loading)
        {
            return;
        }
        self.action_number += 1;
        let number = self.action_number;
        // Each accepted touch keeps its own job. Jobs queue on the shared
        // per-hub FIFO transaction lock; never cancel an acknowledged launch's
        // recency update merely because another launch used the same slot.
        let queued = matches!(key, "project-touch" | "archive-set");
        if !queued && let Some(abort) = self.request_aborts.remove(key) {
            abort.abort();
        }
        let (abort, registration) = AbortHandle::new_pair();
        if !queued {
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
    /// parent deltas are ignored until the reader returns. A transcript held
    /// from an earlier switch shows at once; the read still runs and folds
    /// into it with the same identity-stable keys as a reseed.
    async fn view_child(&mut self, target: Option<ChildTarget>) {
        if target == self.view.child {
            return;
        }
        let other_parent = target
            .as_ref()
            .filter(|t| self.view.selected.as_ref() != Some(&t.parent));
        if other_parent.is_some_and(|t| !self.sessions.contains_key(&t.parent)) {
            return;
        }
        self.hold_transcript();
        if let Some(target) = &target
            && self.view.selected.as_ref() != Some(&target.parent)
        {
            self.select(Some(target.parent.clone())).await;
        }
        let entering = target.is_some();
        let held = self.take_held(target.as_ref());
        self.selection += 1;
        self.view.child = target;
        self.view.loading = held.is_none();
        self.view.loading_older = false;
        if !entering {
            self.conversation_limit = held.as_ref().map_or(CONVERSATION_PAGE, |h| h.limit);
        }
        self.view.transcript = held.map(|h| h.transcript).unwrap_or_default();
        self.conversation_pending = false;
        self.child_pending = false;
        self.buffered.clear();
        self.buffered_bytes = 0;
        if entering {
            self.fetch_child();
        } else {
            self.fetch_conversation();
        }
        self.dirty = true;
    }

    /// Keep the settled transcript being left by a parent/child switch.
    fn hold_transcript(&mut self) {
        if self.view.loading || self.view.transcript.seq.is_none() {
            return;
        }
        let held = Held {
            epoch: self.epoch,
            transcript: self.view.transcript.clone(),
            limit: self.conversation_limit,
        };
        match self.view.child.clone() {
            None => {
                if let Some(id) = self.view.selected.clone() {
                    self.held_parent = Some((id, held));
                }
            }
            Some(child) => {
                self.held_children.retain(|(key, _)| *key != child);
                self.held_children.push_back((child, held));
                while self.held_children.len() > MAX_HELD_CHILDREN
                    || self
                        .held_children
                        .iter()
                        .map(|(_, h)| h.transcript.bytes)
                        .sum::<usize>()
                        > MAX_HELD_BYTES
                {
                    self.held_children.pop_front();
                }
            }
        }
    }

    /// The held transcript for the parent (`None`) or child about to show,
    /// only from this connection epoch and only for that exact owner.
    fn take_held(&mut self, target: Option<&ChildTarget>) -> Option<Held> {
        let epoch = self.epoch;
        self.held_children.retain(|(_, h)| h.epoch == epoch);
        let held = match target {
            Some(target) => {
                let ix = self.held_children.iter().position(|(k, _)| k == target)?;
                self.held_children.remove(ix).map(|(_, h)| h)
            }
            None => self
                .held_parent
                .take()
                .filter(|(id, _)| self.view.selected.as_ref() == Some(id))
                .map(|(_, h)| h),
        };
        held.filter(|h| h.epoch == epoch)
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
        self.subscribe().await;
        self.fetch_conversation();
        self.dirty = true;
    }

    /// Adopt a `sessionArchive` document unless an equal or newer one is
    /// already held (a reply can land after the change event it caused).
    fn apply_archive(&mut self, data: &Value) {
        let Ok(document) = crate::features::archive_document(data.clone()) else {
            return;
        };
        let version = document["version"].as_i64().unwrap_or(0);
        if !self.archive_resync
            && self
                .view
                .session_archive
                .as_ref()
                .is_some_and(|held| held["version"].as_i64().unwrap_or(0) >= version)
        {
            return;
        }
        self.archive_resync = false;
        self.view.session_archive = Some(Arc::new(document));
        self.dirty = true;
    }

    fn upsert(&mut self, data: &Value) {
        let Some(id) = Session::id_of(data) else {
            return;
        };
        if self.is_shell(id) {
            return;
        }
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
        self.terminals_disconnected();
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
                self.archive_resync = true;
                self.view.connected = true;
                self.view.power_paused = false;
                self.view.notice.clear();
                self.fleet_pending = false;
                self.conversation_pending = false;
                self.usage_pending = false;
                self.fetch_fleet();
                self.fetch_usage();
                // Re-read the shared archive on every connect: changes made by
                // another client while this one was away have no replay.
                self.request(crate::features::Request::Archive);
                let child = self.view.child.clone();
                self.select(self.view.selected.clone()).await;
                if child.is_some() {
                    self.view_child(child).await;
                }
                self.reopen_terminals().await;
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
                if let Some(shell) = topic.strip_prefix("pty.bytes.") {
                    self.terminal_bytes(shell, &data).await;
                } else if topic == "pty.exit" || topic == "pty.desync" {
                    if let Some(shell) = data["sessionId"].as_str() {
                        self.terminal_signal(shell, topic == "pty.exit").await;
                    }
                } else if crate::ui_requests::TOPICS.contains(&topic.as_str()) {
                    self.queue_ui_request(&topic, &data);
                } else if topic == "sessionArchive.changed" {
                    self.apply_archive(&data);
                } else if topic == "agent.snapshot" {
                    self.upsert(&data);
                    self.overlay_snapshot(&data);
                } else if self.view.child.is_none()
                    && self
                        .view
                        .selected
                        .as_ref()
                        .is_some_and(|id| topic == format!("agent.conversation.{id}"))
                {
                    self.conversation_event(data);
                }
            }
        }
        self.dirty = true;
    }

    /// A UI request from the bus, queued for the window, or a warning when
    /// it cannot be.
    fn queue_ui_request(&mut self, topic: &str, data: &Value) {
        match crate::ui_requests::parse(topic, data) {
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
    }

    /// Folds a snapshot event into the fleet read in flight, so its reply
    /// cannot roll this newer state back.
    fn overlay_snapshot(&mut self, data: &Value) {
        if self.fleet_pending
            && let Some(id) = Session::id_of(data)
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
            // Child membership and status too, or a fleet read
            // served before this event rolls finished children
            // back to running (or drops new ones) until the next
            // event. Kept as the bounded projection.
            if let Some(session) = self.sessions.get(id) {
                // Clear uses completed tool calls as work evidence.
                // Carry the parsed scalar, not an arbitrary payload,
                // and cover aliases a stale read may have supplied.
                if ["toolCalls", "totalToolCalls", "tool_calls"]
                    .iter()
                    .any(|key| data.get(*key).is_some())
                {
                    for key in ["toolCalls", "totalToolCalls", "tool_calls"] {
                        merged[key] = json!(session.telemetry.tool_calls);
                    }
                }
                for (key, value) in [
                    ("subagents", &session.subagents),
                    ("workflows", &session.workflows),
                ] {
                    if data.get(key).is_some() {
                        merged[key] = value.clone();
                    }
                }
            }
            self.fleet_overlay.insert(id.into(), merged);
        }
    }

    /// A conversation delta for the selected session: applied, buffered
    /// behind a read in flight, or answered with a fresh read.
    fn conversation_event(&mut self, data: Value) {
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
                } else if self.view.transcript.delta(delta, self.streaming()) == Fold::Gap {
                    self.fetch_conversation();
                }
            }
        }
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
                        Action::SetModel { model, effort, .. } => {
                            !model.trim().is_empty()
                                && model.len() < 256
                                && effort.as_deref().is_none_or(crate::launch::valid_effort)
                        }
                        Action::SetEffort(effort) => crate::launch::valid_effort(effort),
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
            anyhow::ensure!(
                self.view.handoff.is_none(),
                "A handoff is preparing a new agent; start this one when it finishes"
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
                    Completion::Spawn(number, request, None, result)
                }));
            }
        }
        self.dirty = true;
    }

    /// Start a handoff: the brief first, then (in `complete`) the successor.
    /// One at a time and never beside a launch, so a repeated click or a
    /// second window cannot start a second successor.
    fn handoff(&mut self, request: crate::handoff::Request) {
        self.action_number += 1;
        let number = self.action_number;
        // Refused with a receipt rather than dropped, so the asking window
        // never waits on a handoff that will not happen. The running one
        // keeps its progress (`finish_handoff` matches by number).
        let checked = if self.view.handoff.is_some() || self.view.creating {
            Err(anyhow!(
                "Another handoff or launch is already in progress; nothing new was started"
            ))
        } else if !self.view.connected {
            Err(anyhow!("Hub disconnected; nothing was started"))
        } else {
            match self.sessions.get(&request.source) {
                Some(source) => request.validate(source),
                None => Err(anyhow!(
                    "The session is no longer listed; nothing was started"
                )),
            }
        };
        if let Err(error) = checked {
            self.finish_handoff(
                number,
                (&request.source, &request.successor.provider),
                None,
                None,
                Some(error.to_string()),
            );
            return;
        }
        self.view.handoff = Some(crate::handoff::Progress {
            number,
            source: request.source.clone(),
            provider: request.successor.provider.clone(),
            stage: crate::handoff::Stage::Brief(request.brief),
        });
        let backend = self.backend.clone();
        self.jobs.push(Box::pin(async move {
            let result = backend.handoff_brief(&request.source, request.brief).await;
            Completion::HandoffBrief(number, request, result)
        }));
        self.dirty = true;
    }

    fn finish_handoff(
        &mut self,
        number: u64,
        (source, provider): (&str, &str),
        successor: Option<String>,
        brief: Option<crate::handoff::Written>,
        error: Option<String>,
    ) {
        if self
            .view
            .handoff
            .as_ref()
            .is_some_and(|p| p.number == number)
        {
            self.view.handoff = None;
        }
        let receipt = crate::handoff::Receipt {
            number,
            source: source.to_owned(),
            provider: provider.to_owned(),
            successor,
            brief,
            error,
        };
        self.view.notice = receipt.summary();
        self.view.handoff_receipt = Some(receipt);
        self.dirty = true;
    }

    async fn complete(&mut self, completion: Completion) {
        match completion {
            Completion::Request(epoch, number, request, result) => {
                let key = request.key();
                if epoch != self.epoch
                    || (!matches!(key, "project-touch" | "archive-set")
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
                if matches!(key, "archive" | "archive-set") && error.is_none() {
                    self.apply_archive(&value);
                }
                if let crate::features::Request::SetArchive { session, archived } = &request {
                    self.view.archive_receipts.push_back(ArchiveReceipt {
                        number,
                        session: session.clone(),
                        archived: *archived,
                        error: error.clone(),
                    });
                    if self.view.archive_receipts.len() > 128 {
                        self.view.archive_receipts.pop_front();
                    }
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
            Completion::HandoffBrief(number, request, result) => {
                if self
                    .view
                    .handoff
                    .as_ref()
                    .is_none_or(|p| p.number != number)
                {
                    return;
                }
                let brief = match result.and_then(|reply| crate::handoff::written(&reply)) {
                    Ok(brief) => brief,
                    Err(error) => {
                        self.finish_handoff(
                            number,
                            (&request.source, &request.successor.provider),
                            None,
                            None,
                            Some(error.to_string()),
                        );
                        return;
                    }
                };
                // Launch checks again: the connection may have dropped while
                // the source agent was writing.
                let params = request.successor.params().and_then(|params| {
                    anyhow::ensure!(self.view.connected, "the hub disconnected");
                    anyhow::ensure!(!self.view.creating, "another launch is in progress");
                    Ok(params)
                });
                match params {
                    Err(error) => {
                        self.finish_handoff(
                            number,
                            (&request.source, &request.successor.provider),
                            None,
                            Some(brief),
                            Some(error.to_string()),
                        );
                    }
                    Ok(params) => {
                        if let Some(progress) = &mut self.view.handoff {
                            progress.stage = crate::handoff::Stage::Starting;
                        }
                        self.view.creating = true;
                        let backend = self.backend.clone();
                        let spawn = HandoffSpawn {
                            source: request.source.clone(),
                            brief,
                        };
                        self.jobs.push(Box::pin(async move {
                            let result = backend.spawn(params).await;
                            Completion::Spawn(number, request.successor, Some(spawn), result)
                        }));
                    }
                }
            }
            Completion::Spawn(number, request, handoff, result) => {
                self.view.creating = false;
                let result = result.and_then(|value| {
                    anyhow::ensure!(value["sessionId"].as_str().is_some_and(|id| !id.is_empty()),
                        "Spawn returned no session ID; outcome unknown. Refresh sessions before retrying");
                    Ok(value)
                });
                match result {
                    Ok(value) => {
                        let id = value["sessionId"].as_str().unwrap().to_owned();
                        // A handoff's takeover message is staged for review,
                        // never sent on the user's behalf.
                        let unsent_message = match &handoff {
                            Some(handoff) => {
                                Some(crate::handoff::successor_prompt(&handoff.brief.path))
                            }
                            None => (!request.message.trim().is_empty()
                                && value["messageQueued"] != true)
                                .then(|| request.message.clone()),
                        };
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
                            session: Some(id.clone()),
                            error: None,
                            unsent_message,
                        });
                        if let Some(handoff) = handoff {
                            self.finish_handoff(
                                number,
                                (&handoff.source, &request.provider),
                                Some(id),
                                Some(handoff.brief),
                                None,
                            );
                        }
                        self.fetch_fleet();
                    }
                    // A failed handoff launch is the handoff's to report; the
                    // New Agent form never shows another flow's error.
                    Err(error) => match handoff {
                        Some(handoff) => self.finish_handoff(
                            number,
                            (&handoff.source, &request.provider),
                            None,
                            Some(handoff.brief),
                            Some(error.to_string()),
                        ),
                        None => {
                            self.view.spawn_receipt = Some(SpawnReceipt {
                                number,
                                session: None,
                                error: Some(error.to_string()),
                                unsent_message: None,
                            });
                        }
                    },
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
                match result {
                    Ok(report) => {
                        self.view.usage = Some(Arc::new(report));
                        self.view.usage_error = None;
                    }
                    Err(error) => {
                        self.view.usage_error =
                            Some(crate::transcript::head(&error.to_string(), 200))
                    }
                }
                self.dirty = true;
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
            Completion::Terminal(epoch, agent, generation, step) => {
                if epoch == self.epoch {
                    self.terminal_done(agent, generation, step).await;
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
                        match &action {
                            Action::SetModel { model, effort, .. } => match effort {
                                Some(effort) => format!(
                                    "Model change accepted: {model} · {} effort",
                                    crate::launch::effort_label(effort)
                                ),
                                None => format!("Model change accepted: {model}"),
                            },
                            Action::SetEffort(effort) => format!(
                                "Effort change accepted: {}",
                                crate::launch::effort_label(effort)
                            ),
                            _ => String::new(),
                        }
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

/// Agent shells: one hub-owned shell per agent, attached while shown.
impl Worker {
    fn is_shell(&self, id: &str) -> bool {
        self.retired_shells.contains(id)
            || self.shells.values().any(|s| s.id.as_deref() == Some(id))
    }

    fn session_list(&self) -> Arc<Vec<Session>> {
        Arc::new(
            self.sessions
                .values()
                .filter(|s| !self.is_shell(&s.id))
                .cloned()
                .collect(),
        )
    }

    /// Selected conversation, UI commands, and every shell being streamed.
    async fn subscribe(&mut self) {
        let mut topics = BTreeSet::from(["agent.snapshot".into(), "sessionArchive.changed".into()]);
        topics.extend(crate::ui_requests::TOPICS.iter().map(|s| s.to_string()));
        if let Some(id) = &self.view.selected {
            topics.insert(format!("agent.conversation.{id}"));
        }
        let mut streaming = false;
        for shell in self.shells.values().filter(|s| s.wanted) {
            if let Some(id) = &shell.id {
                topics.insert(format!("pty.bytes.{id}"));
                streaming = true;
            }
        }
        if streaming {
            topics.insert("pty.exit".into());
            topics.insert("pty.desync".into());
        }
        let _ = self.backend.topics(topics).await;
    }

    fn set_terminal(&mut self, agent: &str, f: impl FnOnce(&mut crate::terminal::Terminal)) {
        let entry = self
            .view
            .terminals
            .entry(agent.to_owned())
            .or_insert_with(|| crate::terminal::Terminal {
                agent: agent.to_owned(),
                ..Default::default()
            });
        f(entry);
        self.dirty = true;
    }

    fn forget_shell_row(&mut self, id: &str) {
        if self.sessions.remove(id).is_some() {
            self.fleet_dirty = true;
        }
    }

    async fn terminal(&mut self, command: crate::terminal::Command) {
        use crate::terminal::{Command as T, Status};
        match command {
            T::Adopt(known) => {
                for (agent, id) in known {
                    if self.shells.contains_key(&agent) || id.is_empty() {
                        continue;
                    }
                    self.shell_generation += 1;
                    self.forget_shell_row(&id);
                    self.shells.insert(
                        agent.clone(),
                        Shell {
                            id: Some(id.clone()),
                            cwd: String::new(),
                            wanted: false,
                            attached: false,
                            generation: self.shell_generation,
                            input: Vec::new(),
                            input_busy: false,
                            size: (crate::terminal::DEFAULT_COLS, crate::terminal::DEFAULT_ROWS),
                            resize_pending: false,
                            resize_busy: false,
                            last_keepalive: Instant::now(),
                        },
                    );
                    self.set_terminal(&agent, |t| {
                        t.shell = Some(id);
                        t.status = Status::Detached;
                    });
                }
            }
            T::Open {
                agent,
                cwd,
                cols,
                rows,
            } => {
                let shell = self.shells.entry(agent.clone()).or_insert_with(|| Shell {
                    id: None,
                    cwd: cwd.clone(),
                    wanted: false,
                    attached: false,
                    generation: 0,
                    input: Vec::new(),
                    input_busy: false,
                    size: (cols, rows),
                    resize_pending: false,
                    resize_busy: false,
                    last_keepalive: Instant::now(),
                });
                shell.wanted = true;
                if shell.cwd.is_empty() {
                    shell.cwd = cwd;
                }
                if shell.size != (cols, rows) {
                    shell.size = (cols, rows);
                    shell.resize_pending = shell.id.is_some();
                }
                if !self.view.connected {
                    self.set_terminal(&agent, |t| {
                        t.status = Status::Failed;
                        t.error = Some("Reconnect to the hub to open a terminal.".into());
                    });
                    return;
                }
                let shell = &self.shells[&agent];
                match (&shell.id, shell.attached) {
                    (Some(_), true) => {}
                    (Some(_), false) => self.attach_shell(&agent).await,
                    (None, _) => self.create_shell(&agent),
                }
                self.flush_resize(&agent);
            }
            T::Hide { agent } => {
                let Some(shell) = self.shells.get_mut(&agent) else {
                    return;
                };
                shell.wanted = false;
                let detach = shell.attached.then(|| shell.id.clone()).flatten();
                shell.attached = false;
                if let Some(id) = detach {
                    let backend = self.backend.clone();
                    // Fire and forget: the lease also lapses on its own.
                    tokio::spawn(async move {
                        let _ = backend
                            .call("sessions.detachTerminal", json!({"sessionId":id}))
                            .await;
                    });
                    self.set_terminal(&agent, |t| {
                        if t.status == Status::Live || t.status == Status::Attaching {
                            t.status = Status::Detached;
                        }
                    });
                }
                self.subscribe().await;
            }
            T::Input { agent, bytes } => {
                let Some(shell) = self.shells.get_mut(&agent) else {
                    return;
                };
                if shell.input.len() + bytes.len() > MAX_TERMINAL_INPUT {
                    self.set_terminal(&agent, |t| {
                        t.error = Some(
                            "Input is arriving faster than the hub accepts it; some was not sent."
                                .into(),
                        );
                    });
                    return;
                }
                shell.input.extend_from_slice(&bytes);
                self.flush_input(&agent);
            }
            T::Resize { agent, cols, rows } => {
                let Some(shell) = self.shells.get_mut(&agent) else {
                    return;
                };
                if shell.size != (cols, rows) {
                    shell.size = (cols, rows);
                    shell.resize_pending = true;
                    self.flush_resize(&agent);
                }
            }
            T::Restart {
                agent,
                cwd,
                cols,
                rows,
            } => {
                if let Some(shell) = self.shells.get_mut(&agent) {
                    if let Some(id) = shell.id.take() {
                        self.view.terminal_feed.forget(&id);
                        self.retired_shells.insert(id.clone());
                        let backend = self.backend.clone();
                        tokio::spawn(async move {
                            let _ = backend
                                .call("claude.signal", json!({"sessionId":id,"signal":"SIGKILL"}))
                                .await;
                        });
                    }
                    shell.cwd = cwd;
                    shell.size = (cols, rows);
                    shell.wanted = true;
                    shell.attached = false;
                    shell.input.clear();
                } else {
                    return Box::pin(self.terminal(T::Open {
                        agent,
                        cwd,
                        cols,
                        rows,
                    }))
                    .await;
                }
                if self.view.connected {
                    self.create_shell(&agent);
                }
                self.subscribe().await;
            }
        }
    }

    fn create_shell(&mut self, agent: &str) {
        use crate::terminal::Status;
        self.shell_generation += 1;
        let generation = self.shell_generation;
        let Some(shell) = self.shells.get_mut(agent) else {
            return;
        };
        shell.generation = generation;
        shell.id = None;
        shell.attached = false;
        shell.input_busy = false;
        shell.resize_busy = false;
        shell.resize_pending = false;
        let params = json!({"cwd":shell.cwd,"cols":shell.size.0,"rows":shell.size.1});
        self.set_terminal(agent, |t| {
            t.status = Status::Starting;
            t.error = None;
            t.shell = None;
        });
        let cwd = self.shells[agent].cwd.clone();
        self.set_terminal(agent, |t| t.cwd = cwd);
        let backend = self.backend.clone();
        let (epoch, agent) = (self.epoch, agent.to_owned());
        self.jobs.push(Box::pin(async move {
            let result = backend.call("terminals.create", params).await;
            Completion::Terminal(epoch, agent, generation, ShellStep::Created(result))
        }));
    }

    async fn attach_shell(&mut self, agent: &str) {
        use crate::terminal::Status;
        let Some(shell) = self.shells.get_mut(agent) else {
            return;
        };
        let Some(id) = shell.id.clone() else {
            return;
        };
        shell.attached = true;
        shell.last_keepalive = Instant::now();
        let generation = shell.generation;
        // Whatever arrives next is the attach's replay of the whole screen.
        self.view.terminal_feed.reset(&id);
        self.set_terminal(agent, |t| {
            t.status = Status::Attaching;
            t.error = None;
            t.shell = Some(id.clone());
            t.attach += 1;
        });
        // Subscribe before attaching: the replay is published immediately.
        self.subscribe().await;
        let backend = self.backend.clone();
        let (epoch, agent) = (self.epoch, agent.to_owned());
        self.jobs.push(Box::pin(async move {
            let result = backend
                .call("sessions.attachTerminal", json!({"sessionId":id}))
                .await;
            Completion::Terminal(epoch, agent, generation, ShellStep::Attached(result))
        }));
    }

    fn flush_input(&mut self, agent: &str) {
        let Some(shell) = self.shells.get_mut(agent) else {
            return;
        };
        let Some(id) = shell.id.clone() else {
            return;
        };
        if shell.input_busy || shell.input.is_empty() || !self.view.connected {
            return;
        }
        shell.input_busy = true;
        let bytes = std::mem::take(&mut shell.input);
        let generation = shell.generation;
        let backend = self.backend.clone();
        let (epoch, agent) = (self.epoch, agent.to_owned());
        self.jobs.push(Box::pin(async move {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let result = backend
                .call(
                    "sessions.terminalInput",
                    json!({"sessionId":id,"bytesB64":encoded}),
                )
                .await;
            Completion::Terminal(epoch, agent, generation, ShellStep::Input(result))
        }));
    }

    fn flush_resize(&mut self, agent: &str) {
        let Some(shell) = self.shells.get_mut(agent) else {
            return;
        };
        let Some(id) = shell.id.clone() else {
            return;
        };
        if shell.resize_busy || !shell.resize_pending || !self.view.connected {
            return;
        }
        shell.resize_busy = true;
        shell.resize_pending = false;
        let (cols, rows) = shell.size;
        let generation = shell.generation;
        let backend = self.backend.clone();
        let (epoch, agent) = (self.epoch, agent.to_owned());
        self.jobs.push(Box::pin(async move {
            let result = backend
                .call(
                    "sessions.terminalResize",
                    json!({"sessionId":id,"cols":cols,"rows":rows}),
                )
                .await;
            {
                let _ = result;
                Completion::Terminal(epoch, agent, generation, ShellStep::Resized)
            }
        }));
    }

    fn keep_terminals_alive(&mut self) {
        let due: Vec<_> = self
            .shells
            .iter_mut()
            .filter(|(_, s)| s.attached && s.last_keepalive.elapsed() >= TERMINAL_KEEPALIVE)
            .filter_map(|(agent, s)| {
                s.last_keepalive = Instant::now();
                Some((agent.clone(), s.id.clone()?, s.generation))
            })
            .collect();
        for (agent, id, generation) in due {
            let backend = self.backend.clone();
            let epoch = self.epoch;
            self.jobs.push(Box::pin(async move {
                let result = backend
                    .call("sessions.terminalKeepalive", json!({"sessionId":id}))
                    .await;
                Completion::Terminal(epoch, agent, generation, ShellStep::Keepalive(result))
            }));
        }
    }

    fn agent_of_shell(&self, id: &str) -> Option<String> {
        self.shells
            .iter()
            .find(|(_, s)| s.id.as_deref() == Some(id))
            .map(|(agent, _)| agent.clone())
    }

    async fn terminal_bytes(&mut self, id: &str, data: &Value) {
        use base64::Engine;
        let Some(agent) = self.agent_of_shell(id) else {
            return;
        };
        if !self.shells[&agent].attached {
            return;
        }
        let Some(bytes) = data
            .as_str()
            .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
        else {
            return;
        };
        if self.view.terminal_feed.push(id, &bytes) {
            self.set_terminal(&agent, |t| {
                if t.status == crate::terminal::Status::Attaching {
                    t.status = crate::terminal::Status::Live;
                }
            });
        } else {
            // The window fell behind; a fresh attach replays the screen.
            self.attach_shell(&agent).await;
        }
    }

    /// `pty.exit` (the shell ended) or `pty.desync` (its stream broke).
    async fn terminal_signal(&mut self, id: &str, exited: bool) {
        let Some(agent) = self.agent_of_shell(id) else {
            return;
        };
        if exited {
            self.shell_ended(&agent, id).await;
        } else if self.shells[&agent].attached {
            self.attach_shell(&agent).await;
        }
    }

    async fn shell_ended(&mut self, agent: &str, id: &str) {
        if let Some(shell) = self.shells.get_mut(agent) {
            shell.id = None;
            shell.attached = false;
            shell.input.clear();
        }
        self.retired_shells.insert(id.to_owned());
        self.set_terminal(agent, |t| {
            t.status = crate::terminal::Status::Exited;
            t.shell = None;
        });
        self.subscribe().await;
    }

    async fn terminal_done(&mut self, agent: String, generation: u64, step: ShellStep) {
        use crate::terminal::Status;
        if self
            .shells
            .get(&agent)
            .is_none_or(|s| s.generation != generation)
        {
            return;
        }
        match step {
            ShellStep::Created(result) => {
                let id = result.and_then(|v| {
                    v["sessionId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow!("The hub started no shell (no session id)."))
                });
                match id {
                    Ok(id) => {
                        self.forget_shell_row(&id);
                        let shell = self.shells.get_mut(&agent).unwrap();
                        shell.id = Some(id.clone());
                        self.set_terminal(&agent, |t| t.shell = Some(id));
                        if self.shells[&agent].wanted {
                            self.attach_shell(&agent).await;
                        } else {
                            self.set_terminal(&agent, |t| t.status = Status::Detached);
                        }
                        self.flush_input(&agent);
                    }
                    Err(error) => self.set_terminal(&agent, |t| {
                        t.status = Status::Failed;
                        t.error = Some(format!("Couldn’t start a terminal: {error}"));
                    }),
                }
            }
            ShellStep::Attached(Ok(_)) => {
                self.set_terminal(&agent, |t| {
                    if t.status == Status::Attaching {
                        t.status = Status::Live;
                    }
                });
                self.flush_resize(&agent);
            }
            ShellStep::Attached(Err(error)) => {
                let message = error.to_string();
                let id = self.shells[&agent].id.clone().unwrap_or_default();
                self.shells.get_mut(&agent).unwrap().attached = false;
                if message.contains("no PTY") || message.contains("not found") {
                    // A remembered shell that no longer exists: start over.
                    self.retired_shells.insert(id.clone());
                    self.view.terminal_feed.forget(&id);
                    if self.shells[&agent].wanted {
                        self.create_shell(&agent);
                    } else {
                        self.shell_ended(&agent, &id).await;
                    }
                } else {
                    self.set_terminal(&agent, |t| {
                        t.status = Status::Failed;
                        t.error = Some(format!("Couldn’t attach to the terminal: {message}"));
                    });
                }
                self.subscribe().await;
            }
            ShellStep::Input(result) => {
                self.shells.get_mut(&agent).unwrap().input_busy = false;
                if let Err(error) = result {
                    self.set_terminal(&agent, |t| {
                        t.error = Some(format!("Input was not delivered: {error}"))
                    });
                } else if self.view.terminals.get(&agent).is_some_and(|t| {
                    t.error
                        .as_deref()
                        .is_some_and(|e| e.starts_with("Input was not delivered"))
                }) {
                    self.set_terminal(&agent, |t| t.error = None);
                }
                self.flush_input(&agent);
            }
            ShellStep::Resized => {
                self.shells.get_mut(&agent).unwrap().resize_busy = false;
                self.flush_resize(&agent);
            }
            ShellStep::Keepalive(Ok(value)) if value["ok"] == false => {
                // The lease lapsed: re-prime the stream with a replay.
                if self.shells[&agent].attached {
                    self.attach_shell(&agent).await;
                }
            }
            ShellStep::Keepalive(_) => {}
        }
    }

    /// After a reconnect, resume every terminal still on screen.
    async fn reopen_terminals(&mut self) {
        let agents: Vec<_> = self
            .shells
            .iter()
            .filter(|(_, s)| s.wanted)
            .map(|(agent, s)| (agent.clone(), s.id.is_some()))
            .collect();
        for (agent, exists) in agents {
            if exists {
                self.attach_shell(&agent).await;
            } else if self
                .view
                .terminals
                .get(&agent)
                .is_none_or(|t| t.status != crate::terminal::Status::Exited)
            {
                self.create_shell(&agent);
            }
        }
    }

    fn terminals_disconnected(&mut self) {
        use crate::terminal::Status;
        let mut changed = Vec::new();
        for (agent, shell) in &mut self.shells {
            shell.attached = false;
            shell.input.clear();
            shell.input_busy = false;
            shell.resize_busy = false;
            if shell.wanted {
                changed.push(agent.clone());
            }
        }
        for agent in changed {
            self.set_terminal(&agent, |t| {
                if t.status != Status::Exited {
                    t.status = Status::Attaching;
                    t.error = Some(
                        "Connection lost. The terminal resumes when the hub reconnects.".into(),
                    );
                }
            });
        }
    }
}
