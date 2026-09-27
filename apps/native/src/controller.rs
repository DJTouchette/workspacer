//! A single background reducer owns network state. The UI receives coalesced,
//! immutable views; it never parses JSON or waits for I/O during rendering.
use anyhow::{Result, anyhow};
use futures_util::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;

use crate::{
    bus::{Client, Config, Event},
    model::{ConversationSnapshot, Delta, Fold, Session, Transcript},
};

const MAX_SESSIONS: usize = 10_000;
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Clone, Debug)]
pub enum Action {
    Send(String),
    Approve(bool),
    Stop,
    Answer(String),
}

impl Action {
    fn wire(&self, id: &str) -> (&'static str, Value) {
        match self {
            Self::Send(text) => ("agents.sendMessage", json!({"sessionId":id, "text":text})),
            Self::Approve(yes) => (
                "claude.approve",
                json!({"sessionId":id, "decision":if *yes {"yes"} else {"no"}}),
            ),
            Self::Stop => ("claude.signal", json!({"sessionId":id, "signal":"SIGINT"})),
            Self::Answer(text) => ("claude.answer", json!({"sessionId":id, "text":text})),
        }
    }
}

pub enum Command {
    Select(String),
    Act { session: String, action: Action },
    Refresh,
}

#[derive(Clone, Debug)]
pub struct Receipt {
    pub number: u64,
    pub session: String,
    pub action: Action,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub connected: bool,
    pub sessions: Arc<Vec<Session>>,
    pub selected: Option<String>,
    pub transcript: Transcript,
    pub loading: bool,
    pub busy: bool,
    pub notice: String,
    pub receipt: Option<Receipt>,
}

pub struct Controller {
    commands: async_channel::Sender<Command>,
    pub views: async_channel::Receiver<Arc<View>>,
}

impl Controller {
    /// Deterministic UI harness: the test supplies server views and observes
    /// commands without a runtime, network, credentials, or agent processes.
    #[cfg(feature = "ui-tests")]
    pub fn test_channels() -> (
        Self,
        async_channel::Receiver<Command>,
        async_channel::Sender<Arc<View>>,
    ) {
        let (commands, observed) = async_channel::bounded(32);
        let (updates, views) = async_channel::bounded(1);
        (Self { commands, views }, observed, updates)
    }

    pub fn start(config: Config) -> Self {
        let (commands, incoming) = async_channel::bounded(32);
        let (updates, views) = async_channel::bounded(1);
        let discard = views.clone();
        tokio::spawn(async move {
            let (bus, events) = Client::start(config);
            Worker::new(bus)
                .run(events, incoming, updates, discard)
                .await;
        });
        Self { commands, views }
    }

    pub fn command(&self, command: Command) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| anyhow!("Client busy; action was not queued"))
    }
}

enum Completion {
    Fleet(u64, Result<Value>),
    Conversation(u64, u64, Result<Value>),
    Action(u64, String, Action, Result<Value>),
}

struct Worker {
    bus: Client,
    view: View,
    sessions: BTreeMap<String, Session>,
    jobs: FuturesUnordered<BoxFuture<'static, Completion>>,
    epoch: u64,
    selection: u64,
    fleet_pending: bool,
    fleet_overlay: BTreeMap<String, Value>,
    conversation_pending: bool,
    buffered: Vec<Delta>,
    buffered_bytes: usize,
    resync_after_read: bool,
    push_ready: bool,
    dirty: bool,
    fleet_dirty: bool,
    action_number: u64,
    last_fleet: Instant,
    last_conversation: Instant,
}

impl Worker {
    fn new(bus: Client) -> Self {
        Self {
            bus,
            view: View::default(),
            sessions: BTreeMap::new(),
            jobs: FuturesUnordered::new(),
            epoch: 0,
            selection: 0,
            fleet_pending: false,
            fleet_overlay: BTreeMap::new(),
            conversation_pending: false,
            buffered: Vec::new(),
            buffered_bytes: 0,
            resync_after_read: false,
            push_ready: false,
            dirty: false,
            fleet_dirty: false,
            action_number: 0,
            last_fleet: Instant::now(),
            last_conversation: Instant::now(),
        }
    }

    async fn run(
        mut self,
        events: async_channel::Receiver<Event>,
        commands: async_channel::Receiver<Command>,
        updates: async_channel::Sender<Arc<View>>,
        discard: async_channel::Receiver<Arc<View>>,
    ) {
        let mut frame = tokio::time::interval(FRAME_INTERVAL);
        frame.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut maintenance = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Err(_) => break,
                    Ok(Command::Select(id)) if self.sessions.contains_key(&id) => self.select(Some(id)).await,
                    Ok(Command::Act { session, action }) => self.act(session, action),
                    Ok(Command::Refresh) => { self.fetch_fleet(); self.fetch_conversation(); }
                    _ => {}
                },
                event = events.recv() => match event {
                    Err(_) => break,
                    Ok(event) => self.event(event).await,
                },
                Some(result) = self.jobs.next(), if !self.jobs.is_empty() => self.complete(result).await,
                _ = frame.tick(), if self.dirty => {
                    if self.fleet_dirty {
                        self.view.sessions = Arc::new(self.sessions.values().cloned().collect());
                        self.fleet_dirty = false;
                    }
                    let view = Arc::new(self.view.clone());
                    // Latest-value mailbox, never an unbounded queue of UI frames.
                    if let Err(async_channel::TrySendError::Full(view)) = updates.try_send(view) {
                        let _ = discard.try_recv();
                        let _ = updates.try_send(view);
                    }
                    self.dirty = false;
                }
                _ = maintenance.tick() => {
                    if self.view.connected {
                        if self.last_fleet.elapsed() >= Duration::from_secs(30) { self.fetch_fleet(); }
                        // Ready suppresses fast polling. A slow reconciliation also
                        // repairs a provider restart behind an otherwise healthy hub.
                        let pace = if self.push_ready { Duration::from_secs(30) } else { Duration::from_secs(1) };
                        if self.last_conversation.elapsed() >= pace { self.fetch_conversation(); }
                    }
                }
            }
        }
        // Dropping jobs and bus closes pending calls and releases subscriptions.
    }

    fn fetch_fleet(&mut self) {
        if !self.view.connected || self.fleet_pending {
            return;
        }
        self.fleet_pending = true;
        self.fleet_overlay.clear();
        self.last_fleet = Instant::now();
        let bus = self.bus.clone();
        let epoch = self.epoch;
        self.jobs.push(Box::pin(async move {
            Completion::Fleet(epoch, bus.call("sessions.snapshots", json!({})).await)
        }));
    }

    fn fetch_conversation(&mut self) {
        if !self.view.connected || self.conversation_pending {
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
        let bus = self.bus.clone();
        let epoch = self.epoch;
        let selection = self.selection;
        self.jobs.push(Box::pin(async move {
            Completion::Conversation(
                epoch,
                selection,
                bus.call("sessions.conversation", json!({"sessionId":id}))
                    .await,
            )
        }));
    }

    async fn select(&mut self, id: Option<String>) {
        self.selection += 1;
        self.view.selected = id;
        self.view.transcript = Transcript::default();
        self.view.loading = self.view.selected.is_some();
        self.conversation_pending = false;
        self.push_ready = false;
        self.buffered.clear();
        let mut topics = BTreeSet::from(["agent.snapshot".into()]);
        if let Some(id) = &self.view.selected {
            topics.insert(format!("agent.conversation.{id}"));
        }
        let _ = self.bus.topics(topics).await;
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

    async fn event(&mut self, event: Event) {
        match event {
            Event::Connected => {
                self.epoch += 1;
                self.view.connected = true;
                self.view.notice.clear();
                self.fleet_pending = false;
                self.conversation_pending = false;
                self.fetch_fleet();
                self.select(self.view.selected.clone()).await;
            }
            Event::Disconnected(reason) => {
                self.epoch += 1;
                self.view.connected = false;
                self.view.loading = false;
                self.view.notice = format!("{reason}. Reconnecting…");
                self.push_ready = false;
            }
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
                if topic == "agent.snapshot" {
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
                } else if self
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
                        Action::Send(text) => !text.trim().is_empty() && text.len() <= 65536,
                        Action::Stop => true,
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
            self.view.busy = true;
            let bus = self.bus.clone();
            self.jobs.push(Box::pin(async move {
                let (method, params) = action.wire(&session);
                Completion::Action(number, session, action, bus.call(method, params).await)
            }));
        }
        self.dirty = true;
    }

    async fn complete(&mut self, completion: Completion) {
        match completion {
            Completion::Fleet(epoch, result) if epoch == self.epoch => {
                self.fleet_pending = false;
                match result {
                    Ok(Value::Array(rows)) => {
                        self.sessions.clear();
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
                    Ok(_) => self.view.notice = "Hub returned an invalid session list".into(),
                    Err(e) => self.view.notice = e.to_string(),
                }
            }
            Completion::Conversation(epoch, selection, result)
                if epoch == self.epoch && selection == self.selection =>
            {
                self.conversation_pending = false;
                self.view.loading = false;
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
                        self.view.transcript.snapshot(snapshot);
                        let streaming = self.streaming();
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
                let error = result.err().map(|e| e.to_string());
                self.view.notice = error.clone().unwrap_or_else(|| "Request accepted".into());
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
