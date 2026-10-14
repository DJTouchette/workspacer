//! Hub-owned per-session outbound message queue.
//!
//! Any client (native, web `/m`, mobile) may queue messages for a session that
//! is busy, edit, remove or reorder them, and ask for one to be sent now. The
//! head entry is delivered (by the session service, through the ordinary
//! `agents.sendMessage` path) once the session has finished a turn and is
//! ready for input, and the next entry waits for the NEXT turn end.
//!
//! This module owns the entries, the per-session turn watch, limits, the
//! persisted document (`message-queues.json` in the hub data dir) and the
//! delivery bookkeeping. Delivery itself is a caller-supplied future, so the
//! whole send protocol is unit tested here.
//!
//! Durability rules:
//! - Every caller-visible mutation is persisted BEFORE it is committed in
//!   memory; a failed write leaves the queue exactly as it was.
//! - An entry is removed (and recorded `inFlight`) before it is sent, so a
//!   crash mid-send can never replay it. A restart surfaces it as
//!   `queueUncertain` instead.
//! - A delivery whose outcome is unknown is never retried: it is surfaced as
//!   `queueUncertain` and the queue pauses until the user acts.
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Entries one session may hold.
pub const MAX_ENTRIES: usize = 50;
/// Bytes of text one entry may hold. Queued entries ride on every
/// `agent.snapshot` and `sessions.snapshots` row, so they are bounded well
/// below the native client's 16 MiB frame.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Bytes of queued text one session may hold in total.
pub const MAX_SESSION_BYTES: usize = 256 * 1024;
/// Bytes of queued text the whole hub may hold (bounds the persisted document).
pub const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
/// Characters of an uncertain entry's text repeated on the snapshot.
const UNCERTAIN_PREVIEW_CHARS: usize = 200;
/// A sent entry whose turn the hub never saw (a turn finished between two
/// observations, or a slash command that runs no turn) counts as done once the
/// session has stayed ready this long. The clock starts when the delivery
/// COMPLETES, and restarts at every observed turn end.
pub const STALE_TURN: Duration = Duration::from_secs(15);

/// One queued message, in the wire shape clients read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub text: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    /// The uuid minted for this entry's delivery attempt and stamped on its
    /// provider frame (Claude stream), so a delivery receipt can settle an
    /// uncertain send. Set only while the entry is in flight; a queued entry
    /// never carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_uuid: Option<String>,
}

/// An entry that may or may not have reached the agent (`queueUncertain`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Uncertain {
    pub id: String,
    /// Truncated preview of the text.
    pub text: String,
    /// Epoch milliseconds.
    pub at: i64,
    /// The uuid the attempt was stamped with. A `started`/`completed`
    /// receipt for exactly this uuid confirms the delivery (see
    /// [`Queues::confirm`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_uuid: Option<String>,
}

/// An uncertain send that a delivery receipt later confirmed
/// (`queueConfirmed`): the agent DID read it. Informational only; it never
/// resumes a paused queue and is cleared by the next resume, send-now,
/// clear, a new uncertain report, or removing its id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmed {
    pub id: String,
    /// Truncated preview of the text.
    pub text: String,
    pub message_uuid: String,
    /// The receipt that confirmed it: `started` or `completed`.
    pub delivery: String,
    /// Epoch milliseconds of the confirmation.
    pub at: i64,
}

/// `sessions.queueUpdate` with `expectedText`: the entry was changed
/// elsewhere since the editor read it. Nothing was changed.
#[derive(Debug)]
pub struct QueueConflict {
    /// The entry's text now.
    pub current: String,
}

impl std::fmt::Display for QueueConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("queued message changed elsewhere")
    }
}

impl std::error::Error for QueueConflict {}

/// The outcome of one delivery attempt.
#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    Sent,
    /// Definitely not delivered (refused before or by the daemon).
    Rejected(String),
    /// May have been delivered; never retried automatically.
    Uncertain(String),
}

/// What a session row says about sending a message right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// At the prompt with no pending approval/question: a send starts a turn.
    Ready,
    /// Producing a turn, compacting or still starting up.
    Busy,
    /// An approval or question is open; never send over it.
    Blocked,
    /// Stopped. The queue is kept until the session is resumed or cleared.
    Ended,
    /// A row owned by another hub; that hub owns its queue.
    Remote,
}

/// Classify a projected (compat) session row.
pub fn readiness(row: &Value) -> Readiness {
    if row["hub"].as_str().is_some_and(|hub| !hub.is_empty()) {
        return Readiness::Remote;
    }
    if super::agent_ops::ended(row) {
        return Readiness::Ended;
    }
    let pending = row.get("pending").is_some_and(|p| !p.is_null());
    if pending || matches!(row["mode"].as_str(), Some("approval" | "question")) {
        return Readiness::Blocked;
    }
    if row["mode"] == "input" && row["compacting"] != true {
        return Readiness::Ready;
    }
    Readiness::Busy
}

/// Watch over the turn(s) a sent entry must wait out before the next entry.
#[derive(Clone, Debug)]
struct Turn {
    /// Turn ends (a ready row after a busy one) still to observe.
    ends_needed: u8,
    /// A busy/blocked row was seen since the last turn end.
    saw_busy: bool,
    /// `user_prompts` at the send or at the last turn end.
    prompts: Option<u64>,
    /// Start of the stale fallback: delivery completion or the last turn end.
    since: Option<Instant>,
}
impl Turn {
    /// Sent while `row` was ready, one turn end is owed. Sent while it was
    /// busy or blocked, the CURRENT turn must end first and then the sent
    /// text's own turn (the daemon holds it until its own input edge).
    fn after_send(row: Option<&Value>, previous: Option<&Turn>) -> Self {
        let ready = row.is_some_and(|row| readiness(row) == Readiness::Ready);
        let base: u8 = if ready { 1 } else { 2 };
        let owed = previous.map_or(0, |p| p.ends_needed.saturating_add(1));
        Self {
            ends_needed: owed.clamp(base, 3),
            saw_busy: !ready,
            prompts: row.and_then(|row| row["user_prompts"].as_u64()),
            since: None,
        }
    }
    /// A watch restored across a restart: only the stale fallback ends it.
    fn restored() -> Self {
        Self {
            ends_needed: 1,
            saw_busy: false,
            prompts: None,
            since: Some(Instant::now()),
        }
    }
    fn delivered(&mut self) {
        if self.since.is_none() {
            self.since = Some(Instant::now());
        }
    }
    /// Feed one row; true when the watched turn(s) are over.
    fn observe(&mut self, row: &Value) -> bool {
        match readiness(row) {
            Readiness::Busy | Readiness::Blocked => {
                self.saw_busy = true;
                false
            }
            Readiness::Ended => true,
            Readiness::Remote => false,
            Readiness::Ready if self.saw_busy => {
                self.saw_busy = false;
                self.ends_needed = self.ends_needed.saturating_sub(1);
                self.prompts = row["user_prompts"].as_u64().or(self.prompts);
                self.since = Some(Instant::now());
                self.ends_needed == 0
            }
            Readiness::Ready => {
                if self.ends_needed > 1 {
                    return false;
                }
                let prompted = match (self.prompts, row["user_prompts"].as_u64()) {
                    (Some(before), Some(now)) => now > before,
                    _ => false,
                };
                prompted
                    || self
                        .since
                        .is_some_and(|since| since.elapsed() >= STALE_TURN)
            }
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Queue {
    #[serde(default)]
    entries: Vec<Entry>,
    #[serde(default)]
    paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uncertain: Option<Uncertain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirmed: Option<Confirmed>,
    /// Removed for delivery, outcome not yet recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    in_flight: Option<Entry>,
    /// Persisted view of the turn watch (the watch itself lives in memory).
    #[serde(default)]
    awaiting_turn: bool,
}
impl Queue {
    fn keep(&self, awaiting: bool) -> bool {
        !self.entries.is_empty()
            || self.paused
            || self.uncertain.is_some()
            || self.confirmed.is_some()
            || self.in_flight.is_some()
            || awaiting
    }
    fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.text.len()).sum()
    }
    /// Lift a pause and its error. Only an explicit resume or a send-now does
    /// this: adding, editing or reordering while paused keeps the queue
    /// paused, so a person decides when sending starts again. An uncertain
    /// report is separate; only its own removal (or a clear) acknowledges it.
    fn resume(&mut self) {
        self.paused = false;
        self.error = None;
        self.confirmed = None;
    }
    fn wire(&self) -> Value {
        serde_json::to_value(&self.entries).unwrap_or_else(|_| json!([]))
    }
    fn position(&self, entry: &str) -> Result<usize> {
        self.entries
            .iter()
            .position(|e| e.id == entry)
            .ok_or_else(|| anyhow!("queued message not found"))
    }
    fn mark_uncertain(&mut self, entry: &Entry, error: String) {
        self.uncertain = Some(Uncertain {
            id: entry.id.clone(),
            text: entry.text.chars().take(UNCERTAIN_PREVIEW_CHARS).collect(),
            at: chrono::Utc::now().timestamp_millis(),
            message_uuid: entry.message_uuid.clone(),
        });
        self.confirmed = None;
        self.paused = true;
        self.error = Some(error);
    }
}

fn check_size(session: usize, others: usize) -> Result<()> {
    if session > MAX_SESSION_BYTES {
        bail!("queued messages for this session would exceed {MAX_SESSION_BYTES} bytes");
    }
    if session + others > MAX_TOTAL_BYTES {
        bail!("the hub's message queues are full ({MAX_TOTAL_BYTES} bytes)");
    }
    Ok(())
}

#[derive(Deserialize, Default)]
struct Document {
    #[serde(default)]
    sessions: BTreeMap<String, Queue>,
}

type Lock = Arc<tokio::sync::Mutex<()>>;

/// Per-session queues plus their persisted document.
pub struct Queues {
    path: Option<PathBuf>,
    state: Mutex<BTreeMap<String, Queue>>,
    turns: Mutex<BTreeMap<String, Turn>>,
    /// Serializes every queue mutation AND delivery for one session, so a
    /// send-now racing the auto-flush cannot send the same entry twice.
    locks: Mutex<BTreeMap<String, Lock>>,
    /// Orders document writes and the commits that follow them.
    writer: tokio::sync::Mutex<()>,
    kicks: tokio::sync::mpsc::UnboundedSender<String>,
    receiver: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<String>>>,
    /// Per session, the message uuids its latest row reports as read by the
    /// agent (`messageReceipts` in state `started`/`completed`) → that state.
    delivered: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
}

/// The uuids a row's `messageReceipts` report as read by the agent.
fn delivered_uuids(row: &Value) -> BTreeMap<String, String> {
    row["messageReceipts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|receipt| {
            let uuid = receipt["uuid"].as_str()?;
            let state = receipt["state"].as_str()?;
            matches!(state, "started" | "completed").then(|| (uuid.to_owned(), state.to_owned()))
        })
        .collect()
}

impl Default for Queues {
    fn default() -> Self {
        Self::with_path(None, BTreeMap::new())
    }
}

impl Queues {
    fn with_path(path: Option<PathBuf>, state: BTreeMap<String, Queue>) -> Self {
        let (kicks, receiver) = tokio::sync::mpsc::unbounded_channel();
        let turns = state
            .iter()
            .filter(|(_, queue)| queue.awaiting_turn)
            .map(|(id, _)| (id.clone(), Turn::restored()))
            .collect();
        Self {
            path,
            state: Mutex::new(state),
            turns: Mutex::new(turns),
            locks: Default::default(),
            writer: tokio::sync::Mutex::new(()),
            kicks,
            receiver: Mutex::new(Some(receiver)),
            delivered: Default::default(),
        }
    }
    /// Open the persisted document. An unreadable document is reported and
    /// replaced on the next write rather than refusing hub startup. An entry
    /// that was in flight when the hub stopped is surfaced as uncertain and
    /// never resent.
    pub fn open(path: PathBuf) -> Self {
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Document>(&bytes) {
                Ok(document) => document
                    .sessions
                    .into_iter()
                    .filter(|(id, _)| valid_id(id))
                    .map(|(id, mut queue)| {
                        if let Some(entry) = queue.in_flight.take() {
                            queue.mark_uncertain(
                                &entry,
                                "delivery was interrupted by a hub restart; it may or may not have reached the agent".into(),
                            );
                        }
                        (id, queue)
                    })
                    .collect(),
                Err(error) => {
                    eprintln!("message queues at {} are unreadable: {error}", path.display());
                    BTreeMap::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                eprintln!("message queues at {} are unreadable: {error}", path.display());
                BTreeMap::new()
            }
        };
        Self::with_path(Some(path), state)
    }
    /// The flush worker's wakeups (taken once).
    pub fn take_kicks(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<String>> {
        self.receiver.lock().unwrap().take()
    }
    pub fn session_lock(&self, id: &str) -> Lock {
        self.locks
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default()
            .clone()
    }
    /// The `queue` reply value for one session.
    pub fn list(&self, id: &str) -> Value {
        self.state
            .lock()
            .unwrap()
            .get(id)
            .map(Queue::wire)
            .unwrap_or_else(|| json!([]))
    }
    /// Sessions that currently hold entries.
    pub fn pending_sessions(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, queue)| !queue.entries.is_empty())
            .map(|(id, _)| id.clone())
            .collect()
    }
    /// Sessions with any queue record.
    pub fn sessions(&self) -> Vec<String> {
        self.state.lock().unwrap().keys().cloned().collect()
    }
    /// Snapshot fields for a projected row. Rows of sessions that never had a
    /// queue are untouched; once a session has one, `queuedMessages` stays on
    /// its rows (empty when drained) so clients merging rows see it clear.
    pub fn enrich(&self, mut row: Value) -> Value {
        let Some(id) = row["sessionId"].as_str().map(str::to_owned) else {
            return row;
        };
        let state = self.state.lock().unwrap();
        let Some(queue) = state.get(&id) else {
            return row;
        };
        if let Some(object) = row.as_object_mut() {
            object.insert("queuedMessages".into(), queue.wire());
            object.insert("queuePaused".into(), json!(queue.paused));
            match &queue.error {
                Some(error) => object.insert("queueError".into(), json!(error)),
                None => object.remove("queueError"),
            };
            match &queue.uncertain {
                Some(uncertain) => object.insert("queueUncertain".into(), json!(uncertain)),
                None => object.remove("queueUncertain"),
            };
            match &queue.confirmed {
                Some(confirmed) => object.insert("queueConfirmed".into(), json!(confirmed)),
                None => object.remove("queueConfirmed"),
            };
        }
        row
    }
    /// Feed an observed row into its session's turn watch and wake the flush
    /// worker when the head entry may now be sent.
    pub fn observe(&self, row: &Value) {
        let Some(id) = row["sessionId"]
            .as_str()
            .or_else(|| row["session_id"].as_str())
        else {
            return;
        };
        {
            let mut turns = self.turns.lock().unwrap();
            if let Some(turn) = turns.get_mut(id)
                && turn.observe(row)
            {
                turns.remove(id);
            }
        }
        let delivered = delivered_uuids(row);
        let confirmable = self
            .uncertain_uuid(id)
            .is_some_and(|u| delivered.contains_key(&u));
        // Only sessions with a queue record can have a send to settle; every
        // other row leaves nothing behind.
        let queued = self.state.lock().unwrap().contains_key(id);
        let mut known = self.delivered.lock().unwrap();
        if queued && !delivered.is_empty() {
            known.insert(id.to_owned(), delivered);
        } else {
            known.remove(id);
        }
        drop(known);
        // The worker settles a confirmable report under the session lock.
        if confirmable || self.wants_flush(id, row) {
            self.kick(id);
        }
    }
    fn uncertain_uuid(&self, id: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .get(id)
            .and_then(|queue| queue.uncertain.as_ref())
            .and_then(|uncertain| uncertain.message_uuid.clone())
    }
    /// The receipt state the latest row reports for `uuid`, when the agent
    /// has read that message.
    fn delivered_state(&self, id: &str, uuid: Option<&str>) -> Option<String> {
        let uuid = uuid?;
        self.delivered.lock().unwrap().get(id)?.get(uuid).cloned()
    }
    /// Settle the uncertain report when a delivery receipt for EXACTLY its
    /// uuid says the agent read the message: the report becomes
    /// `queueConfirmed`. Nothing is resent, and a paused queue stays paused
    /// (only `sessions.queueResume` / `queueSendNow` lift a pause). Returns
    /// whether anything changed. Callers hold `session_lock(id)`.
    pub async fn confirm(&self, id: &str) -> bool {
        let uuid = self.uncertain_uuid(id);
        let Some(delivery) = self.delivered_state(id, uuid.as_deref()) else {
            return false;
        };
        let uuid = uuid.unwrap_or_default();
        let mut changed = false;
        self.commit(id, |queue| {
            if let Some(uncertain) = queue
                .uncertain
                .take_if(|u| u.message_uuid.as_deref() == Some(uuid.as_str()))
            {
                queue.confirmed = Some(Confirmed {
                    id: uncertain.id,
                    text: uncertain.text,
                    message_uuid: uuid.clone(),
                    delivery: delivery.clone(),
                    at: chrono::Utc::now().timestamp_millis(),
                });
                changed = true;
            }
        })
        .await;
        changed
    }
    fn wants_flush(&self, id: &str, row: &Value) -> bool {
        let sendable = self.state.lock().unwrap().get(id).is_some_and(|queue| {
            !queue.entries.is_empty()
                && !queue.paused
                && queue.uncertain.is_none()
                && queue.in_flight.is_none()
        });
        sendable
            && !self.turns.lock().unwrap().contains_key(id)
            && readiness(row) == Readiness::Ready
    }
    pub fn kick(&self, id: &str) {
        let _ = self.kicks.send(id.to_owned());
    }
    fn drop_lock(&self, id: &str) {
        let mut locks = self.locks.lock().unwrap();
        // Never discard a lock somebody holds or waits on.
        if locks
            .get(id)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            locks.remove(id);
        }
    }
    /// Drop queues of sessions the hub no longer knows at all.
    pub async fn prune(&self, keep: &BTreeSet<String>) -> Result<bool> {
        let _writer = self.writer.lock().await;
        let removed: Vec<String> = self
            .state
            .lock()
            .unwrap()
            .keys()
            .filter(|id| !keep.contains(*id))
            .cloned()
            .collect();
        if removed.is_empty() {
            return Ok(false);
        }
        for id in &removed {
            self.state.lock().unwrap().remove(id);
            self.turns.lock().unwrap().remove(id);
            self.delivered.lock().unwrap().remove(id);
            self.drop_lock(id);
        }
        self.write(self.document(None)?).await?;
        Ok(true)
    }
    /// Drop one session's queue (closed/dismissed).
    pub async fn forget(&self, id: &str) -> Result<()> {
        let _writer = self.writer.lock().await;
        let existed = self.state.lock().unwrap().remove(id).is_some();
        self.turns.lock().unwrap().remove(id);
        self.delivered.lock().unwrap().remove(id);
        self.drop_lock(id);
        if existed {
            self.write(self.document(None)?).await?;
        }
        Ok(())
    }

    // ---- mutations: callers hold `session_lock(id)` ----

    pub async fn add(&self, id: &str, text: &str) -> Result<String> {
        validate_text(text)?;
        self.transact(id, |queue, others| {
            if queue.entries.len() >= MAX_ENTRIES {
                bail!("message queue is full ({MAX_ENTRIES} entries)");
            }
            check_size(queue.bytes() + text.len(), others)?;
            let entry = Entry {
                id: format!("q-{}", uuid::Uuid::new_v4().simple()),
                text: text.to_owned(),
                created_at: chrono::Utc::now().timestamp_millis(),
                message_uuid: None,
            };
            let entry_id = entry.id.clone();
            queue.entries.push(entry);
            Ok(entry_id)
        })
        .await
    }
    /// Replace an entry's text. With `expected`, the edit applies only while
    /// the entry still reads exactly that (what the editor started from):
    /// a change made elsewhere meanwhile is a [`QueueConflict`] and nothing
    /// changes, so one client's edit never silently overwrites another's.
    pub async fn update(
        &self,
        id: &str,
        entry: &str,
        text: &str,
        expected: Option<&str>,
    ) -> Result<()> {
        validate_text(text)?;
        self.transact(id, |queue, others| {
            let index = queue.position(entry)?;
            if let Some(expected) = expected
                && queue.entries[index].text != expected
            {
                return Err(QueueConflict {
                    current: queue.entries[index].text.clone(),
                }
                .into());
            }
            let old = queue.entries[index].text.len();
            check_size(queue.bytes() - old + text.len(), others)?;
            queue.entries[index].text = text.to_owned();
            Ok(())
        })
        .await
    }
    /// Idempotent: removing an absent entry succeeds. Removing the uncertain
    /// report's id acknowledges it; a pause stays until resumed.
    pub async fn remove(&self, id: &str, entry: &str) -> Result<()> {
        if !self.state.lock().unwrap().contains_key(id) {
            return Ok(());
        }
        self.transact(id, |queue, _| {
            queue.entries.retain(|e| e.id != entry);
            if queue.uncertain.as_ref().is_some_and(|u| u.id == entry) {
                queue.uncertain = None;
            }
            if queue.confirmed.as_ref().is_some_and(|c| c.id == entry) {
                queue.confirmed = None;
            }
            Ok(())
        })
        .await
    }
    /// Lift a pause (`sessions.queueResume`); the worker then sends the head
    /// at the next turn end as usual. Resuming an unpaused queue is a no-op.
    pub async fn resume_sending(&self, id: &str) -> Result<()> {
        if !self
            .state
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|queue| queue.paused || queue.error.is_some())
        {
            return Ok(());
        }
        self.transact(id, |queue, _| {
            queue.resume();
            Ok(())
        })
        .await
    }
    pub async fn move_to(&self, id: &str, entry: &str, index: usize) -> Result<()> {
        self.transact(id, |queue, _| {
            let from = queue.position(entry)?;
            if index >= queue.entries.len() {
                bail!(
                    "index must be between 0 and {}",
                    queue.entries.len().saturating_sub(1)
                );
            }
            let moved = queue.entries.remove(from);
            queue.entries.insert(index, moved);
            Ok(())
        })
        .await
    }
    /// Empty the queue entirely: entries, the uncertain report and the pause
    /// (with nothing left there is nothing for a pause to hold back).
    pub async fn clear(&self, id: &str) -> Result<()> {
        if !self.state.lock().unwrap().contains_key(id) {
            return Ok(());
        }
        self.transact(id, |queue, _| {
            queue.entries.clear();
            queue.uncertain = None;
            queue.resume();
            Ok(())
        })
        .await
    }

    // ---- delivery: callers hold `session_lock(id)` ----

    /// Send the head entry if `row` allows it now. Returns None when nothing
    /// was attempted.
    pub async fn flush<F, Fut>(&self, id: &str, row: &Value, deliver: F) -> Option<Delivery>
    where
        F: FnOnce(Entry) -> Fut,
        Fut: Future<Output = Delivery>,
    {
        if !self.wants_flush(id, row) {
            return None;
        }
        let head = match self
            .transact(id, |queue, _| {
                if queue.entries.is_empty() {
                    bail!("message queue is empty");
                }
                let mut head = queue.entries.remove(0);
                head.message_uuid = Some(uuid::Uuid::new_v4().hyphenated().to_string());
                queue.in_flight = Some(head.clone());
                Ok(head)
            })
            .await
        {
            Ok(head) => head,
            Err(error) => {
                // Not removed, so not sent: pause until the user acts.
                let message = format!("the queue could not be saved, so nothing was sent: {error}");
                self.commit(id, |queue| {
                    queue.paused = true;
                    queue.error = Some(message.clone());
                })
                .await;
                return Some(Delivery::Rejected(message));
            }
        };
        self.turns
            .lock()
            .unwrap()
            .insert(id.to_owned(), Turn::after_send(Some(row), None));
        let outcome = self.upgrade(id, &head, deliver(head.clone()).await);
        self.settle(id, 0, head, &outcome, true, None).await;
        Some(outcome)
    }
    /// Remove one named entry and deliver it immediately.
    pub async fn send_now<F, Fut>(
        &self,
        id: &str,
        entry: &str,
        row: Option<&Value>,
        deliver: F,
    ) -> Result<()>
    where
        F: FnOnce(Entry) -> Fut,
        Fut: Future<Output = Delivery>,
    {
        let (index, taken) = self
            .transact(id, |queue, _| {
                let index = queue.position(entry)?;
                let mut taken = queue.entries.remove(index);
                taken.message_uuid = Some(uuid::Uuid::new_v4().hyphenated().to_string());
                queue.resume();
                queue.in_flight = Some(taken.clone());
                Ok((index, taken))
            })
            .await?;
        let previous = {
            let mut turns = self.turns.lock().unwrap();
            let previous = turns.remove(id);
            turns.insert(id.to_owned(), Turn::after_send(row, previous.as_ref()));
            previous
        };
        let outcome = self.upgrade(id, &taken, deliver(taken.clone()).await);
        self.settle(id, index, taken, &outcome, false, previous)
            .await;
        match outcome {
            Delivery::Sent => Ok(()),
            Delivery::Rejected(error) => Err(anyhow!(error)),
            Delivery::Uncertain(error) => Err(anyhow!(
                "message delivery outcome is unknown; it was not retried: {error}"
            )),
        }
    }
    /// An uncertain outcome whose receipt already arrived (the agent read
    /// this attempt's uuid before its acknowledgement was lost) was a send.
    fn upgrade(&self, id: &str, entry: &Entry, outcome: Delivery) -> Delivery {
        match outcome {
            Delivery::Uncertain(_)
                if self
                    .delivered_state(id, entry.message_uuid.as_deref())
                    .is_some() =>
            {
                Delivery::Sent
            }
            outcome => outcome,
        }
    }
    /// Record a delivery outcome. Rejected: the entry goes back where it was
    /// (the auto-flush also pauses). Uncertain: the entry is NOT restored; it
    /// is reported as `queueUncertain` and the queue pauses.
    async fn settle(
        &self,
        id: &str,
        index: usize,
        entry: Entry,
        outcome: &Delivery,
        auto: bool,
        previous: Option<Turn>,
    ) {
        match outcome {
            Delivery::Sent => {
                if let Some(turn) = self.turns.lock().unwrap().get_mut(id) {
                    turn.delivered();
                }
                self.commit(id, |queue| queue.in_flight = None).await;
            }
            Delivery::Rejected(error) => {
                {
                    let mut turns = self.turns.lock().unwrap();
                    turns.remove(id);
                    if let Some(previous) = previous {
                        turns.insert(id.to_owned(), previous);
                    }
                }
                self.commit(id, |queue| {
                    queue.in_flight = None;
                    let index = index.min(queue.entries.len());
                    // Never sent: the next attempt gets a fresh uuid.
                    let mut entry = entry;
                    entry.message_uuid = None;
                    queue.entries.insert(index, entry);
                    if auto {
                        queue.paused = true;
                        queue.error = Some(error.clone());
                    }
                })
                .await;
            }
            Delivery::Uncertain(error) => {
                if let Some(turn) = self.turns.lock().unwrap().get_mut(id) {
                    turn.delivered();
                }
                self.commit(id, |queue| {
                    queue.in_flight = None;
                    queue.mark_uncertain(&entry, error.clone());
                })
                .await;
            }
        }
    }

    // ---- persistence ----

    /// Apply `f` to a copy of the session's queue, persist the result and
    /// only then commit it. `f` also receives the bytes other sessions hold.
    async fn transact<R>(
        &self,
        id: &str,
        f: impl FnOnce(&mut Queue, usize) -> Result<R>,
    ) -> Result<R> {
        let _writer = self.writer.lock().await;
        let (mut candidate, others) = {
            let state = self.state.lock().unwrap();
            let others = state
                .iter()
                .filter(|(key, _)| key.as_str() != id)
                .map(|(_, queue)| queue.bytes())
                .sum();
            (state.get(id).cloned().unwrap_or_default(), others)
        };
        let result = f(&mut candidate, others)?;
        let document = self.document(Some((id, &candidate)))?;
        self.write(document).await?;
        self.state.lock().unwrap().insert(id.to_owned(), candidate);
        Ok(result)
    }
    /// Commit in memory, then persist best-effort (bookkeeping after a send
    /// attempt must not be lost just because the disk refused it).
    async fn commit(&self, id: &str, f: impl FnOnce(&mut Queue)) {
        let _writer = self.writer.lock().await;
        f(self.state.lock().unwrap().entry(id.to_owned()).or_default());
        let written = match self.document(None) {
            Ok(document) => self.write(document).await,
            Err(error) => Err(error),
        };
        if let Err(error) = written {
            eprintln!("message queue was not persisted: {error}");
        }
    }
    /// The document for the committed state, with `candidate` in place of
    /// its session when given.
    fn document(&self, candidate: Option<(&str, &Queue)>) -> Result<Value> {
        let state = self.state.lock().unwrap();
        let turns = self.turns.lock().unwrap();
        let mut sessions = serde_json::Map::new();
        let mut put = |id: &str, queue: &Queue| -> Result<()> {
            let awaiting = turns.contains_key(id);
            if queue.keep(awaiting) {
                let mut queue = queue.clone();
                queue.awaiting_turn = awaiting;
                sessions.insert(id.to_owned(), serde_json::to_value(&queue)?);
            }
            Ok(())
        };
        for (id, queue) in state.iter() {
            if candidate.is_none_or(|(skip, _)| skip != id) {
                put(id, queue)?;
            }
        }
        if let Some((id, queue)) = candidate {
            put(id, queue)?;
        }
        Ok(json!({"version":1,"sessions":sessions}))
    }
    async fn write(&self, document: Value) -> Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || super::atomic_json(&path, &document, true)).await?
    }
}

pub fn valid_id(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 128
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

fn validate_text(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("queued message text must not be empty");
    }
    if text.len() > MAX_TEXT_BYTES {
        bail!("queued message text exceeds {MAX_TEXT_BYTES} bytes");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mode: &str) -> Value {
        row_for("s1", mode)
    }
    fn row_for(id: &str, mode: &str) -> Value {
        json!({"sessionId":id,"session_id":id,"mode":mode,
            "status":if mode=="stopped"{"ended"}else{"active"},"pending":null,"user_prompts":1})
    }
    fn texts(queues: &Queues, id: &str) -> Vec<String> {
        queues
            .list(id)
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                e.as_str()
                    .map_or_else(|| e["text"].as_str().unwrap().to_owned(), str::to_owned)
            })
            .collect()
    }
    /// A deliverer that records what it was handed and answers `outcome`.
    fn deliverer(
        log: &Arc<Mutex<Vec<String>>>,
        outcome: Delivery,
    ) -> impl FnOnce(Entry) -> std::future::Ready<Delivery> {
        let log = log.clone();
        move |entry| {
            log.lock().unwrap().push(entry.text);
            std::future::ready(outcome)
        }
    }
    fn uncertain(queues: &Queues, id: &str) -> Value {
        queues.enrich(json!({"sessionId":id}))["queueUncertain"].clone()
    }

    #[test]
    fn readiness_reads_mode_pending_and_ownership() {
        assert_eq!(readiness(&row("input")), Readiness::Ready);
        assert_eq!(readiness(&row("responding")), Readiness::Busy);
        assert_eq!(readiness(&row("unknown")), Readiness::Busy);
        assert_eq!(readiness(&row("approval")), Readiness::Blocked);
        assert_eq!(readiness(&row("question")), Readiness::Blocked);
        assert_eq!(readiness(&row("stopped")), Readiness::Ended);
        let mut pending = row("input");
        pending["pending"] = json!({"kind":"approval"});
        assert_eq!(readiness(&pending), Readiness::Blocked);
        let mut compacting = row("input");
        compacting["compacting"] = json!(true);
        assert_eq!(readiness(&compacting), Readiness::Busy);
        let mut remote = row("input");
        remote["hub"] = json!("work");
        assert_eq!(readiness(&remote), Readiness::Remote);
    }

    #[tokio::test]
    async fn ops_validate_ids_indices_and_idempotent_removal() {
        let queues = Queues::default();
        assert!(queues.add("s1", "  ").await.is_err());
        let a = queues.add("s1", "a").await.unwrap();
        let b = queues.add("s1", "b").await.unwrap();
        let c = queues.add("s1", "c").await.unwrap();
        assert_eq!(texts(&queues, "s1"), ["a", "b", "c"]);
        assert!(queues.list("s1")[0]["createdAt"].as_i64().unwrap() > 0);
        queues.move_to("s1", &c, 0).await.unwrap();
        assert_eq!(texts(&queues, "s1"), ["c", "a", "b"]);
        assert!(queues.move_to("s1", &c, 3).await.is_err());
        assert!(queues.move_to("s1", "q-missing", 0).await.is_err());
        assert!(queues.move_to("other", &c, 0).await.is_err());
        queues.update("s1", &a, "a2", None).await.unwrap();
        assert!(queues.update("s1", "q-missing", "x", None).await.is_err());
        assert!(queues.update("s1", &a, "", None).await.is_err());
        queues.remove("s1", &b).await.unwrap();
        queues.remove("s1", &b).await.unwrap();
        assert_eq!(texts(&queues, "s1"), ["c", "a2"]);
        // A session without a queue does not grow one from a remove/clear.
        queues.remove("nobody", &b).await.unwrap();
        queues.clear("nobody").await.unwrap();
        assert!(
            queues
                .enrich(json!({"sessionId":"nobody"}))
                .get("queuedMessages")
                .is_none()
        );
        queues.clear("s1").await.unwrap();
        assert_eq!(queues.list("s1"), json!([]));
        assert!(valid_id("abc-1_2") && !valid_id("") && !valid_id("../x") && !valid_id("a.b"));
    }

    #[tokio::test]
    async fn size_limits_bound_entries_sessions_and_the_hub() {
        let queues = Queues::default();
        assert!(
            queues
                .add("s1", &"x".repeat(MAX_TEXT_BYTES + 1))
                .await
                .is_err()
        );
        let chunk = "x".repeat(MAX_TEXT_BYTES);
        for _ in 0..MAX_SESSION_BYTES / MAX_TEXT_BYTES {
            queues.add("s1", &chunk).await.unwrap();
        }
        let error = queues.add("s1", "y").await.unwrap_err().to_string();
        assert!(error.contains("exceed"), "{error}");
        // Updates are held to the same per-session total.
        let first = queues.list("s1")[0]["id"].as_str().unwrap().to_owned();
        queues.update("s1", &first, "small", None).await.unwrap();
        assert!(queues.update("s1", &first, &chunk, None).await.is_ok());
        let small = Queues::default();
        for n in 0..MAX_ENTRIES {
            small.add("s1", &format!("m{n}")).await.unwrap();
        }
        assert!(small.add("s1", "overflow").await.is_err());
        // Hub-wide total.
        let hub = Queues::default();
        let sessions = MAX_TOTAL_BYTES / MAX_SESSION_BYTES;
        for s in 0..sessions {
            for _ in 0..MAX_SESSION_BYTES / MAX_TEXT_BYTES {
                hub.add(&format!("s{s}"), &chunk).await.unwrap();
            }
        }
        let error = hub.add("extra", "y").await.unwrap_err().to_string();
        assert!(error.contains("full"), "{error}");
    }

    #[test]
    fn a_turn_sent_at_the_prompt_ends_after_busy_then_ready() {
        let mut turn = Turn::after_send(Some(&row("input")), None);
        assert!(
            !turn.observe(&row("input")),
            "the sent turn has not started"
        );
        turn.delivered();
        assert!(!turn.observe(&row("responding")));
        assert!(!turn.observe(&row("question")));
        assert!(turn.observe(&row("input")));
        let mut prompted = Turn::after_send(Some(&row("input")), None);
        let mut later = row("input");
        later["user_prompts"] = json!(2);
        assert!(
            prompted.observe(&later),
            "a counted prompt ends a missed turn"
        );
        let mut stopped = Turn::after_send(Some(&row("input")), None);
        assert!(stopped.observe(&row("stopped")));
    }

    #[test]
    fn a_turn_sent_while_busy_waits_out_the_current_turn_and_its_own() {
        let mut turn = Turn::after_send(Some(&row("responding")), None);
        turn.delivered();
        // A delivery that completed long ago does not start the clock: the
        // held turn's end restarts it.
        turn.since = Some(Instant::now() - STALE_TURN);
        assert!(!turn.observe(&row("responding")));
        // The held turn ends: the sent text's own turn is still owed.
        assert!(!turn.observe(&row("input")));
        assert!(!turn.observe(&row("input")), "measured from the turn end");
        assert!(!turn.observe(&row("responding")));
        assert!(turn.observe(&row("input")));
        // Measured from that ready edge, the stale fallback applies again.
        let mut missed = Turn::after_send(Some(&row("approval")), None);
        assert!(!missed.observe(&row("input")));
        missed.since = Some(Instant::now() - STALE_TURN);
        assert!(missed.observe(&row("input")));
    }

    #[test]
    fn the_stale_fallback_starts_when_delivery_completes() {
        let mut turn = Turn::after_send(Some(&row("input")), None);
        assert!(turn.since.is_none());
        assert!(
            !turn.observe(&row("input")),
            "no clock before delivery completes"
        );
        turn.delivered();
        assert!(!turn.observe(&row("input")));
        turn.since = Some(Instant::now() - STALE_TURN);
        assert!(turn.observe(&row("input")));
    }

    #[tokio::test]
    async fn flush_sends_the_head_once_and_waits_for_the_next_turn() {
        let queues = Queues::default();
        let log = Arc::default();
        queues.add("s1", "one").await.unwrap();
        queues.add("s1", "two").await.unwrap();
        for mode in ["responding", "approval", "question", "stopped", "unknown"] {
            assert!(
                queues
                    .flush("s1", &row(mode), deliverer(&log, Delivery::Sent))
                    .await
                    .is_none()
            );
        }
        assert_eq!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await,
            Some(Delivery::Sent)
        );
        assert!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none()
        );
        queues.observe(&row("responding"));
        assert!(
            queues
                .flush("s1", &row("question"), deliverer(&log, Delivery::Sent))
                .await
                .is_none()
        );
        queues.observe(&row("input"));
        queues
            .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
            .await
            .unwrap();
        assert_eq!(*log.lock().unwrap(), ["one", "two"]);
        assert_eq!(texts(&queues, "s1"), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_rejected_auto_send_restores_and_pauses_until_resumed() {
        let queues = Queues::default();
        let log = Arc::default();
        queues.add("s1", "one").await.unwrap();
        let outcome = queues
            .flush(
                "s1",
                &row("input"),
                deliverer(&log, Delivery::Rejected("refused".into())),
            )
            .await;
        assert_eq!(outcome, Some(Delivery::Rejected("refused".into())));
        let enriched = queues.enrich(json!({"sessionId":"s1"}));
        assert_eq!(enriched["queuePaused"], true);
        assert_eq!(enriched["queueError"], "refused");
        assert!(enriched.get("queueUncertain").is_none());
        assert_eq!(texts(&queues, "s1"), ["one"]);
        assert!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none()
        );
        // Adding, editing and reordering keep it paused.
        let two = queues.add("s1", "two").await.unwrap();
        queues.update("s1", &two, "two!", None).await.unwrap();
        queues.move_to("s1", &two, 0).await.unwrap();
        queues.move_to("s1", &two, 1).await.unwrap();
        let paused = queues.enrich(json!({"sessionId":"s1"}));
        assert_eq!(paused["queuePaused"], true);
        assert_eq!(paused["queueError"], "refused");
        assert!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none()
        );
        queues.resume_sending("s1").await.unwrap();
        let resumed = queues.enrich(json!({"sessionId":"s1"}));
        assert_eq!(resumed["queuePaused"], false);
        assert!(resumed.get("queueError").is_none());
        queues
            .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
            .await
            .unwrap();
        assert_eq!(*log.lock().unwrap(), ["one", "one"]);
        assert_eq!(texts(&queues, "s1"), ["two!"]);
        // Resuming an unpaused queue (or one never seen) is a no-op.
        queues.resume_sending("s1").await.unwrap();
        queues.resume_sending("nobody").await.unwrap();
    }

    #[tokio::test]
    async fn an_edit_from_stale_text_is_a_conflict_and_changes_nothing() {
        let queues = Queues::default();
        let id = queues.add("s1", "original").await.unwrap();
        queues
            .update("s1", &id, "theirs", Some("original"))
            .await
            .unwrap();
        let error = queues
            .update("s1", &id, "mine", Some("original"))
            .await
            .unwrap_err();
        let conflict = error
            .downcast_ref::<QueueConflict>()
            .expect("a typed conflict");
        assert_eq!(conflict.current, "theirs");
        assert_eq!(error.to_string(), "queued message changed elsewhere");
        assert_eq!(texts(&queues, "s1"), ["theirs"]);
        // Without expectedText the last write wins, as before.
        queues.update("s1", &id, "mine", None).await.unwrap();
        assert_eq!(texts(&queues, "s1"), ["mine"]);
    }

    #[tokio::test]
    async fn clear_empties_entries_report_and_pause() {
        let queues = Queues::default();
        let log = Arc::default();
        queues.add("s1", "one").await.unwrap();
        queues.add("s1", "two").await.unwrap();
        queues
            .flush(
                "s1",
                &row("input"),
                deliverer(&log, Delivery::Uncertain("lost".into())),
            )
            .await
            .unwrap();
        queues.clear("s1").await.unwrap();
        let row = queues.enrich(json!({"sessionId":"s1"}));
        assert_eq!(row["queuedMessages"], json!([]), "{row}");
        assert_eq!(row["queuePaused"], false);
        assert!(row.get("queueUncertain").is_none());
    }

    #[tokio::test]
    async fn an_uncertain_auto_send_is_never_retried() {
        let queues = Queues::default();
        let log = Arc::default();
        queues.add("s1", "one").await.unwrap();
        queues.add("s1", "two").await.unwrap();
        queues
            .flush(
                "s1",
                &row("input"),
                deliverer(&log, Delivery::Uncertain("ack lost".into())),
            )
            .await
            .unwrap();
        assert_eq!(texts(&queues, "s1"), ["two"], "not restored");
        let report = uncertain(&queues, "s1");
        assert_eq!(report["text"], "one");
        assert!(report["id"].as_str().unwrap().starts_with("q-"));
        assert!(report["at"].as_i64().unwrap() > 0);
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"}))["queuePaused"],
            true
        );
        // Paused even after the turn ends.
        queues.observe(&row("responding"));
        queues.observe(&row("input"));
        assert!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none()
        );
        // Other changes neither acknowledge the report nor resume sending.
        queues.add("s1", "three").await.unwrap();
        assert_eq!(uncertain(&queues, "s1")["text"], "one");
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"}))["queuePaused"],
            true
        );
        // Acknowledged by an idempotent remove of the uncertain id; the
        // pause holds until an explicit resume.
        let id = report["id"].as_str().unwrap().to_owned();
        queues.remove("s1", &id).await.unwrap();
        assert!(uncertain(&queues, "s1").is_null());
        assert!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none(),
            "dismissing the report does not resume"
        );
        queues.resume_sending("s1").await.unwrap();
        queues
            .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
            .await
            .unwrap();
        assert_eq!(*log.lock().unwrap(), ["one", "two"]);
    }

    #[tokio::test]
    async fn send_now_outcomes() {
        let queues = Queues::default();
        let log = Arc::default();
        let one = queues.add("s1", "one").await.unwrap();
        let two = queues.add("s1", "two").await.unwrap();
        let three = queues.add("s1", "three").await.unwrap();
        // Rejected: restored where it was, not paused, error returned.
        let error = queues
            .send_now(
                "s1",
                &two,
                Some(&row("responding")),
                deliverer(&log, Delivery::Rejected("no".into())),
            )
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "no");
        assert_eq!(texts(&queues, "s1"), ["one", "two", "three"]);
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"}))["queuePaused"],
            false
        );
        // Uncertain: not restored, reported, paused.
        let error = queues
            .send_now(
                "s1",
                &three,
                Some(&row("input")),
                deliverer(&log, Delivery::Uncertain("lost".into())),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unknown"), "{error}");
        assert_eq!(texts(&queues, "s1"), ["one", "two"]);
        assert_eq!(uncertain(&queues, "s1")["text"], "three");
        assert!(
            queues
                .send_now("s1", &three, None, deliverer(&log, Delivery::Sent))
                .await
                .is_err()
        );
        // Sent while busy (a held turn): the next entry waits for the current
        // turn AND the sent text's own turn.
        queues
            .send_now(
                "s1",
                &one,
                Some(&row("responding")),
                deliverer(&log, Delivery::Sent),
            )
            .await
            .unwrap();
        // A send now resumes sending but leaves the may-have-been-sent report
        // for the user to acknowledge.
        assert_eq!(uncertain(&queues, "s1")["text"], "three");
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"}))["queuePaused"],
            false
        );
        queues.observe(&row("responding"));
        queues.observe(&row("input"));
        assert!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none(),
            "flushed in the same turn end as the sent-now text"
        );
        queues.observe(&row("responding"));
        queues.observe(&row("input"));
        // The unacknowledged report still holds the automatic send back.
        assert!(
            queues
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none()
        );
        queues.remove("s1", &three).await.unwrap();
        queues
            .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
            .await
            .unwrap();
        assert_eq!(*log.lock().unwrap(), ["two", "three", "one", "two"]);
    }

    #[tokio::test]
    async fn persist_failures_change_nothing_and_send_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, b"").unwrap();
        let entry = |id: &str, text: &str| Entry {
            message_uuid: None,
            id: id.into(),
            text: text.into(),
            created_at: 1,
        };
        let queues = Queues::with_path(
            Some(blocker.join("message-queues.json")),
            [(
                "s1".to_owned(),
                Queue {
                    entries: vec![entry("q-1", "one"), entry("q-2", "two")],
                    ..Default::default()
                },
            )]
            .into(),
        );
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        assert!(queues.add("s1", "three").await.is_err());
        assert!(queues.add("s2", "three").await.is_err());
        assert!(queues.update("s1", "q-1", "edited", None).await.is_err());
        assert!(queues.move_to("s1", "q-2", 0).await.is_err());
        assert!(queues.remove("s1", "q-1").await.is_err());
        assert!(queues.clear("s1").await.is_err());
        assert_eq!(texts(&queues, "s1"), ["one", "two"]);
        assert!(
            queues
                .enrich(json!({"sessionId":"s2"}))
                .get("queuedMessages")
                .is_none()
        );
        assert!(
            queues
                .send_now(
                    "s1",
                    "q-2",
                    Some(&row("input")),
                    deliverer(&log, Delivery::Sent)
                )
                .await
                .is_err()
        );
        let outcome = queues
            .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
            .await;
        assert!(matches!(outcome, Some(Delivery::Rejected(_))));
        assert!(log.lock().unwrap().is_empty(), "nothing may be sent");
        assert_eq!(texts(&queues, "s1"), ["one", "two"]);
        let row = queues.enrich(json!({"sessionId":"s1"}));
        assert_eq!(row["queuePaused"], true);
        assert!(
            row["queueError"]
                .as_str()
                .unwrap()
                .contains("nothing was sent")
        );
    }

    #[tokio::test]
    async fn persistence_round_trips_prunes_and_surfaces_interrupted_sends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("message-queues.json");
        let queues = Queues::open(path.clone());
        let log = Arc::default();
        queues.add("keep", "one").await.unwrap();
        queues.add("keep", "two").await.unwrap();
        queues
            .flush(
                "keep",
                &row_for("keep", "input"),
                deliverer(&log, Delivery::Rejected("boom".into())),
            )
            .await
            .unwrap();
        queues.add("drop", "x").await.unwrap();
        queues.add("empty", "x").await.unwrap();
        queues.clear("empty").await.unwrap();
        let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["version"], 1);
        assert!(raw["sessions"].get("empty").is_none(), "{raw}");
        assert!(raw["sessions"]["keep"].get("inFlight").is_none());
        let reopened = Queues::open(path.clone());
        assert_eq!(reopened.list("keep"), queues.list("keep"));
        let enriched = reopened.enrich(json!({"sessionId":"keep"}));
        assert_eq!(enriched["queuePaused"], true);
        assert_eq!(enriched["queueError"], "boom");
        assert!(reopened.prune(&["keep".to_owned()].into()).await.unwrap());
        assert_eq!(reopened.list("drop"), json!([]));
        let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(raw["sessions"].get("drop").is_none());

        // A hub that stopped mid-send left an inFlight entry: it comes back as
        // uncertain and is never resent.
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"version":1,"sessions":{"s1":{
                "entries":[{"id":"q-2","text":"two","createdAt":2}],
                "inFlight":{"id":"q-1","text":"one","createdAt":1},
                "awaitingTurn":true}}}))
            .unwrap(),
        )
        .unwrap();
        let restarted = Queues::open(path.clone());
        let report = uncertain(&restarted, "s1");
        assert_eq!(report["id"], "q-1");
        assert_eq!(report["text"], "one");
        assert_eq!(texts(&restarted, "s1"), ["two"]);
        assert!(
            restarted
                .flush("s1", &row("input"), deliverer(&log, Delivery::Sent))
                .await
                .is_none()
        );
        std::fs::write(&path, b"{not json").unwrap();
        assert!(Queues::open(path).sessions().is_empty());
    }

    #[tokio::test]
    async fn a_held_lock_survives_forget_and_prune() {
        let queues = Queues::default();
        queues.add("s1", "one").await.unwrap();
        let lock = queues.session_lock("s1");
        let guard = lock.lock().await;
        queues.forget("s1").await.unwrap();
        assert!(Arc::ptr_eq(&lock, &queues.session_lock("s1")));
        drop(guard);
        queues.add("s2", "one").await.unwrap();
        queues.prune(&BTreeSet::new()).await.unwrap();
        assert!(queues.sessions().is_empty());
    }

    /// A deliverer that records the uuid each attempt was stamped with.
    fn stamped(
        uuids: &Arc<Mutex<Vec<Option<String>>>>,
        outcome: Delivery,
    ) -> impl FnOnce(Entry) -> std::future::Ready<Delivery> {
        let uuids = uuids.clone();
        move |entry| {
            uuids.lock().unwrap().push(entry.message_uuid);
            std::future::ready(outcome)
        }
    }
    fn with_receipts(mut row: Value, receipts: Value) -> Value {
        row["messageReceipts"] = receipts;
        row
    }

    #[tokio::test]
    async fn a_receipt_for_the_exact_uuid_confirms_an_uncertain_send_without_resuming() {
        let queues = Queues::default();
        let uuids = Arc::new(Mutex::new(Vec::new()));
        queues.add("s1", "one").await.unwrap();
        queues.add("s1", "two").await.unwrap();
        queues
            .flush(
                "s1",
                &row("input"),
                stamped(&uuids, Delivery::Uncertain("ack lost".into())),
            )
            .await
            .unwrap();
        let uuid = uuids.lock().unwrap()[0]
            .clone()
            .expect("every attempt is stamped");
        assert_eq!(uncertain(&queues, "s1")["messageUuid"], uuid.as_str());
        // A queued entry never carries a uuid on the wire.
        assert!(queues.list("s1")[0].get("messageUuid").is_none());

        // Another message's receipt, or this one merely queued, settles nothing.
        queues.observe(&with_receipts(
            row("responding"),
            json!([{"uuid":"someone-else","state":"completed","at":1},{"uuid":uuid,"state":"queued","at":1}]),
        ));
        assert!(!queues.confirm("s1").await);
        assert_eq!(uncertain(&queues, "s1")["text"], "one");

        // Its own `started` receipt does: the report becomes a confirmation,
        // the pause and the remaining entry stay, nothing is resent.
        queues.observe(&with_receipts(
            row("responding"),
            json!([{"uuid":uuid,"state":"started","at":2}]),
        ));
        assert!(queues.confirm("s1").await);
        assert!(!queues.confirm("s1").await, "idempotent");
        let row_out = queues.enrich(json!({"sessionId":"s1"}));
        assert!(row_out.get("queueUncertain").is_none());
        assert_eq!(row_out["queueConfirmed"]["id"], uncertain_id(&row_out));
        assert_eq!(row_out["queueConfirmed"]["delivery"], "started");
        assert_eq!(row_out["queueConfirmed"]["messageUuid"], uuid.as_str());
        assert_eq!(
            row_out["queuePaused"], true,
            "a receipt never lifts a pause"
        );
        assert_eq!(texts(&queues, "s1"), ["two"]);
        assert_eq!(uuids.lock().unwrap().len(), 1);
        assert!(
            queues
                .flush("s1", &row("input"), stamped(&uuids, Delivery::Sent))
                .await
                .is_none()
        );

        // Resuming is still the person's call, and it clears the note.
        queues.resume_sending("s1").await.unwrap();
        let row_out = queues.enrich(json!({"sessionId":"s1"}));
        assert!(row_out.get("queueConfirmed").is_none());
        assert_eq!(row_out["queuePaused"], false);
    }
    fn uncertain_id(row: &Value) -> Value {
        row["queueConfirmed"]["id"].clone()
    }

    #[tokio::test]
    async fn an_uncertain_send_already_receipted_counts_as_sent() {
        let queues = Arc::new(Queues::default());
        queues.add("s1", "one").await.unwrap();
        // The receipt lands while the send is still waiting on its ack.
        let seen = queues.clone();
        let outcome = queues
            .flush("s1", &row("input"), move |entry| {
                let uuid = entry.message_uuid.clone().unwrap();
                seen.observe(
                    &json!({"sessionId":"s1","session_id":"s1","mode":"responding","pending":null,
                    "messageReceipts":[{"uuid":uuid,"state":"completed","at":1}]}),
                );
                std::future::ready(Delivery::Uncertain("ack lost".into()))
            })
            .await;
        assert_eq!(outcome, Some(Delivery::Sent));
        assert!(uncertain(&queues, "s1").is_null());
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"}))["queuePaused"],
            false
        );
    }

    #[tokio::test]
    async fn a_rejected_attempt_is_restamped_on_retry() {
        let queues = Queues::default();
        let uuids = Arc::new(Mutex::new(Vec::new()));
        let id = queues.add("s1", "one").await.unwrap();
        queues
            .send_now(
                "s1",
                &id,
                Some(&row("input")),
                stamped(&uuids, Delivery::Rejected("no".into())),
            )
            .await
            .unwrap_err();
        assert!(
            queues.list("s1")[0].get("messageUuid").is_none(),
            "restored without its uuid"
        );
        queues
            .send_now(
                "s1",
                &id,
                Some(&row("input")),
                stamped(&uuids, Delivery::Sent),
            )
            .await
            .unwrap();
        let uuids = uuids.lock().unwrap();
        assert!(uuids[0].is_some() && uuids[1].is_some() && uuids[0] != uuids[1]);
    }

    #[tokio::test]
    async fn receipts_are_kept_only_for_queued_sessions_and_dropped_with_them() {
        let queues = Queues::default();
        let receipts = json!([{"uuid":"u1","state":"completed","at":1}]);
        queues.observe(&with_receipts(row_for("never", "input"), receipts.clone()));
        assert!(
            queues.delivered.lock().unwrap().is_empty(),
            "no queue, nothing kept"
        );
        queues.add("s1", "one").await.unwrap();
        queues.observe(&with_receipts(row("input"), receipts.clone()));
        assert!(queues.delivered.lock().unwrap().contains_key("s1"));
        queues.forget("s1").await.unwrap();
        assert!(queues.delivered.lock().unwrap().is_empty());
        queues.add("s2", "two").await.unwrap();
        queues.observe(&with_receipts(row_for("s2", "input"), receipts));
        queues.prune(&BTreeSet::new()).await.unwrap();
        assert!(queues.delivered.lock().unwrap().is_empty());
    }
}
