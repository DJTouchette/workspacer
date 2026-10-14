//! Hub-owned per-session outbound message queue.
//!
//! Any client (native, web `/m`, mobile) may queue messages for a session that
//! is busy, edit, remove or reorder them, and ask for one to be sent now. The
//! session service (`sessions::queue`) owns delivery: it sends the head entry
//! through the ordinary `agents.sendMessage` path once the session has finished
//! a turn and is ready for input, then waits for the NEXT turn end before
//! sending the next entry.
//!
//! This module is the store: entries, the per-session turn watch, limits and
//! the persisted document (`message-queues.json` in the hub data dir). It does
//! no delivery and publishes nothing itself.
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Entries one session may hold.
pub const MAX_ENTRIES: usize = 50;
/// Bytes of text one entry may hold. `agents.sendMessage` itself is bounded by
/// the daemon's 16 MiB request body; a queued entry is also persisted and
/// re-published on every snapshot, so it is held to a tighter bound.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// Serialized bytes of one entry's optional attachment list.
pub const MAX_ATTACHMENT_BYTES: usize = 64 * 1024;
/// Attachments one entry may name.
pub const MAX_ATTACHMENTS: usize = 20;
/// A sent entry whose session never visibly went busy (a fast turn the hub
/// missed between two observations, or a slash command that runs no turn) is
/// treated as done after the session has stayed ready this long.
pub const STALE_TURN: Duration = Duration::from_secs(15);

/// One queued message, in the wire shape clients read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub text: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Value>,
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

/// Watch over the turn a sent entry started.
#[derive(Clone, Debug)]
struct Turn {
    saw_busy: bool,
    prompts: Option<u64>,
    since: Instant,
}
impl Turn {
    fn started(row: Option<&Value>) -> Self {
        Self {
            saw_busy: false,
            prompts: row.and_then(|row| row["user_prompts"].as_u64()),
            since: Instant::now(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Queue {
    #[serde(default)]
    entries: Vec<Entry>,
    #[serde(default)]
    paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// A sent entry's turn has not been seen to end yet.
    #[serde(default)]
    awaiting_turn: bool,
    #[serde(skip)]
    turn: Option<Turn>,
}
impl Queue {
    fn idle(&self) -> bool {
        self.entries.is_empty() && !self.paused && !self.awaiting_turn
    }
    fn resume(&mut self) {
        self.paused = false;
        self.error = None;
    }
    fn wire(&self) -> Value {
        serde_json::to_value(&self.entries).unwrap_or_else(|_| json!([]))
    }
    /// Feed one observed row into the turn watch.
    fn observe(&mut self, row: &Value) {
        if !self.awaiting_turn {
            return;
        }
        let turn = self.turn.get_or_insert_with(|| Turn::started(None));
        match readiness(row) {
            Readiness::Busy | Readiness::Blocked => turn.saw_busy = true,
            // The turn the sent entry started is over either way; a resumed
            // life starts with a fresh prompt.
            Readiness::Ended => {
                self.awaiting_turn = false;
                self.turn = None;
            }
            Readiness::Ready => {
                let prompted = match (turn.prompts, row["user_prompts"].as_u64()) {
                    (Some(before), Some(now)) => now > before,
                    _ => false,
                };
                if turn.saw_busy || prompted || turn.since.elapsed() >= STALE_TURN {
                    self.awaiting_turn = false;
                    self.turn = None;
                }
            }
            Readiness::Remote => {}
        }
    }
    fn wants_flush(&self, row: &Value) -> bool {
        !self.entries.is_empty()
            && !self.paused
            && !self.awaiting_turn
            && readiness(row) == Readiness::Ready
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Document {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    sessions: BTreeMap<String, Queue>,
}

/// Per-session queues plus their persisted document.
pub struct Queues {
    path: Option<PathBuf>,
    state: Mutex<BTreeMap<String, Queue>>,
    /// Serializes every queue mutation AND delivery for one session, so a
    /// send-now racing the auto-flush cannot send the same entry twice.
    locks: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Orders document writes: each write snapshots the state it holds the
    /// lock for, so the last write always carries the newest state.
    writer: tokio::sync::Mutex<()>,
    kicks: tokio::sync::mpsc::UnboundedSender<String>,
    receiver: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<String>>>,
}

impl Default for Queues {
    fn default() -> Self {
        Self::with_path(None, BTreeMap::new())
    }
}

impl Queues {
    fn with_path(path: Option<PathBuf>, state: BTreeMap<String, Queue>) -> Self {
        let (kicks, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            path,
            state: Mutex::new(state),
            locks: Default::default(),
            writer: tokio::sync::Mutex::new(()),
            kicks,
            receiver: Mutex::new(Some(receiver)),
        }
    }
    /// Open the persisted document. An unreadable document is reported and
    /// replaced on the next write rather than refusing hub startup.
    pub fn open(path: PathBuf) -> Self {
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Document>(&bytes) {
                Ok(document) => document
                    .sessions
                    .into_iter()
                    .filter(|(id, queue)| valid_id(id) && !queue.idle())
                    .map(|(id, mut queue)| {
                        queue.entries.truncate(MAX_ENTRIES);
                        if queue.awaiting_turn {
                            // Only the stale fallback can vouch for a turn that
                            // was in flight across a restart.
                            queue.turn = Some(Turn::started(None));
                        }
                        (id, queue)
                    })
                    .collect(),
                Err(error) => {
                    eprintln!(
                        "message queues at {} are unreadable: {error}",
                        path.display()
                    );
                    BTreeMap::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                eprintln!(
                    "message queues at {} are unreadable: {error}",
                    path.display()
                );
                BTreeMap::new()
            }
        };
        Self::with_path(Some(path), state)
    }
    /// The flush worker's wakeups (taken once).
    pub fn take_kicks(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<String>> {
        self.receiver.lock().unwrap().take()
    }
    pub fn session_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
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
        let wake = {
            let mut state = self.state.lock().unwrap();
            let Some(queue) = state.get_mut(id) else {
                return;
            };
            queue.observe(row);
            queue.wants_flush(row)
        };
        if wake {
            self.kick(id);
        }
    }
    pub fn kick(&self, id: &str) {
        let _ = self.kicks.send(id.to_owned());
    }
    /// Drop queues of sessions the hub no longer retains.
    pub fn retain(&self, ids: &BTreeSet<String>) -> bool {
        let mut state = self.state.lock().unwrap();
        let before = state.len();
        state.retain(|id, _| ids.contains(id));
        self.locks.lock().unwrap().retain(|id, _| ids.contains(id));
        state.len() != before
    }
    /// Drop one session's queue (closed/dismissed).
    pub fn forget(&self, id: &str) -> bool {
        self.locks.lock().unwrap().remove(id);
        self.state.lock().unwrap().remove(id).is_some()
    }

    // ---- mutations: callers hold `session_lock(id)` ----

    pub fn add(&self, id: &str, text: &str, attachments: Option<Value>) -> Result<String> {
        validate_text(text)?;
        let attachments = validate_attachments(attachments)?;
        let mut state = self.state.lock().unwrap();
        let queue = state.entry(id.to_owned()).or_default();
        if queue.entries.len() >= MAX_ENTRIES {
            bail!("message queue is full ({MAX_ENTRIES} entries)");
        }
        let entry = Entry {
            id: format!("q-{}", uuid::Uuid::new_v4().simple()),
            text: text.to_owned(),
            created_at: chrono::Utc::now().timestamp_millis(),
            attachments,
        };
        let entry_id = entry.id.clone();
        queue.entries.push(entry);
        queue.resume();
        Ok(entry_id)
    }
    pub fn update(&self, id: &str, entry: &str, text: &str) -> Result<()> {
        validate_text(text)?;
        let mut state = self.state.lock().unwrap();
        let queue = state.get_mut(id).ok_or_else(missing)?;
        let found = queue
            .entries
            .iter_mut()
            .find(|e| e.id == entry)
            .ok_or_else(missing)?;
        found.text = text.to_owned();
        queue.resume();
        Ok(())
    }
    /// Idempotent: removing an absent entry succeeds.
    pub fn remove(&self, id: &str, entry: &str) {
        if let Some(queue) = self.state.lock().unwrap().get_mut(id) {
            queue.entries.retain(|e| e.id != entry);
            queue.resume();
        }
    }
    pub fn move_to(&self, id: &str, entry: &str, index: usize) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        let queue = state.get_mut(id).ok_or_else(missing)?;
        let from = queue
            .entries
            .iter()
            .position(|e| e.id == entry)
            .ok_or_else(missing)?;
        if index >= queue.entries.len() {
            bail!(
                "index must be between 0 and {}",
                queue.entries.len().saturating_sub(1)
            );
        }
        let moved = queue.entries.remove(from);
        queue.entries.insert(index, moved);
        queue.resume();
        Ok(())
    }
    pub fn clear(&self, id: &str) {
        if let Some(queue) = self.state.lock().unwrap().get_mut(id) {
            queue.entries.clear();
            queue.resume();
        }
    }
    /// Take one named entry for an explicit send. Returns it with its index so
    /// a failed send can put it back where it was.
    pub fn take(&self, id: &str, entry: &str, row: Option<&Value>) -> Result<(usize, Entry)> {
        let mut state = self.state.lock().unwrap();
        let queue = state.get_mut(id).ok_or_else(missing)?;
        let index = queue
            .entries
            .iter()
            .position(|e| e.id == entry)
            .ok_or_else(missing)?;
        let taken = queue.entries.remove(index);
        queue.resume();
        queue.awaiting_turn = true;
        queue.turn = Some(Turn::started(row));
        Ok((index, taken))
    }
    /// Take the head entry for the auto-flush, if the row says it may go now.
    pub fn take_head(&self, id: &str, row: &Value) -> Option<Entry> {
        let mut state = self.state.lock().unwrap();
        let queue = state.get_mut(id)?;
        queue.observe(row);
        if !queue.wants_flush(row) {
            return None;
        }
        let head = queue.entries.remove(0);
        queue.awaiting_turn = true;
        queue.turn = Some(Turn::started(Some(row)));
        Some(head)
    }
    /// A send failed: put the entry back. The auto-flush pauses with the
    /// error until the user acts; an explicit send-now just restores it.
    pub fn restore(&self, id: &str, index: usize, entry: Entry, pause: Option<String>) {
        let mut state = self.state.lock().unwrap();
        let queue = state.entry(id.to_owned()).or_default();
        let index = index.min(queue.entries.len());
        queue.entries.insert(index, entry);
        queue.entries.truncate(MAX_ENTRIES);
        queue.awaiting_turn = false;
        queue.turn = None;
        if let Some(error) = pause {
            queue.paused = true;
            queue.error = Some(error);
        }
    }

    /// Write the document (only sessions with something to keep).
    pub async fn persist(&self) -> Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let _writer = self.writer.lock().await;
        let document = {
            let state = self.state.lock().unwrap();
            let sessions: serde_json::Map<String, Value> = state
                .iter()
                .filter(|(_, queue)| !queue.idle())
                .map(|(id, queue)| Ok((id.clone(), serde_json::to_value(queue)?)))
                .collect::<Result<_>>()?;
            json!({"version":1,"sessions":sessions})
        };
        tokio::task::spawn_blocking(move || super::atomic_json(&path, &document, true)).await?
    }
}

fn missing() -> anyhow::Error {
    anyhow!("queued message not found")
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

fn validate_attachments(attachments: Option<Value>) -> Result<Option<Value>> {
    match attachments {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.is_empty() => Ok(None),
        Some(Value::Array(items)) => {
            if items.len() > MAX_ATTACHMENTS {
                bail!("a queued message may carry at most {MAX_ATTACHMENTS} attachments");
            }
            let value = Value::Array(items);
            if serde_json::to_vec(&value)?.len() > MAX_ATTACHMENT_BYTES {
                bail!("queued message attachments exceed {MAX_ATTACHMENT_BYTES} bytes");
            }
            Ok(Some(value))
        }
        Some(_) => bail!("attachments must be an array"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mode: &str) -> Value {
        json!({"sessionId":"s1","session_id":"s1","mode":mode,"status":if mode=="stopped"{"ended"}else{"active"},"pending":null,"user_prompts":1})
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

    #[test]
    fn ops_validate_limits_ids_and_indices() {
        let queues = Queues::default();
        assert!(queues.add("s1", "  ", None).is_err());
        assert!(
            queues
                .add("s1", &"x".repeat(MAX_TEXT_BYTES + 1), None)
                .is_err()
        );
        assert!(queues.add("s1", "a", Some(json!("nope"))).is_err());
        assert!(
            queues
                .add("s1", "a", Some(json!(vec![json!(1); MAX_ATTACHMENTS + 1])))
                .is_err()
        );
        let a = queues.add("s1", "a", None).unwrap();
        let b = queues
            .add("s1", "b", Some(json!([{"path":"/tmp/x.png"}])))
            .unwrap();
        let c = queues.add("s1", "c", Some(json!([]))).unwrap();
        let ids = |q: &Queues| -> Vec<String> {
            q.list("s1")
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["text"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(ids(&queues), ["a", "b", "c"]);
        let listed = queues.list("s1");
        assert_eq!(listed[1]["attachments"], json!([{"path":"/tmp/x.png"}]));
        assert!(listed[2].get("attachments").is_none());
        assert!(listed[0]["createdAt"].as_i64().unwrap() > 0);
        queues.move_to("s1", &c, 0).unwrap();
        assert_eq!(ids(&queues), ["c", "a", "b"]);
        assert!(queues.move_to("s1", &c, 3).is_err());
        assert!(queues.move_to("s1", "q-missing", 0).is_err());
        assert!(queues.move_to("other", &c, 0).is_err());
        queues.update("s1", &a, "a2").unwrap();
        assert!(queues.update("s1", "q-missing", "x").is_err());
        assert!(queues.update("s1", &a, "").is_err());
        queues.remove("s1", &b);
        queues.remove("s1", &b);
        queues.remove("nobody", &b);
        assert_eq!(ids(&queues), ["c", "a2"]);
        queues.clear("s1");
        assert_eq!(queues.list("s1"), json!([]));
        for n in 0..MAX_ENTRIES {
            queues.add("s1", &format!("m{n}"), None).unwrap();
        }
        assert!(queues.add("s1", "overflow", None).is_err());
        assert!(valid_id("abc-1_2") && !valid_id("") && !valid_id("../x") && !valid_id("a.b"));
    }

    #[test]
    fn head_waits_for_ready_and_the_next_turn_end() {
        let queues = Queues::default();
        queues.add("s1", "one", None).unwrap();
        queues.add("s1", "two", None).unwrap();
        assert!(queues.take_head("s1", &row("responding")).is_none());
        assert!(queues.take_head("s1", &row("approval")).is_none());
        assert!(queues.take_head("s1", &row("stopped")).is_none());
        assert_eq!(queues.take_head("s1", &row("input")).unwrap().text, "one");
        // Still at the prompt before the sent turn begins: the next waits.
        assert!(queues.take_head("s1", &row("input")).is_none());
        queues.observe(&row("responding"));
        assert!(queues.take_head("s1", &row("approval")).is_none());
        assert_eq!(queues.take_head("s1", &row("input")).unwrap().text, "two");
        assert!(queues.take_head("s1", &row("input")).is_none());
    }

    #[test]
    fn a_counted_prompt_ends_a_turn_the_hub_never_saw_busy() {
        let queues = Queues::default();
        queues.add("s1", "one", None).unwrap();
        queues.add("s1", "two", None).unwrap();
        assert!(queues.take_head("s1", &row("input")).is_some());
        let mut later = row("input");
        later["user_prompts"] = json!(2);
        assert_eq!(queues.take_head("s1", &later).unwrap().text, "two");
    }

    #[test]
    fn a_stop_ends_the_watched_turn_and_keeps_the_queue() {
        let queues = Queues::default();
        queues.add("s1", "one", None).unwrap();
        queues.add("s1", "two", None).unwrap();
        assert!(queues.take_head("s1", &row("input")).is_some());
        queues.observe(&row("stopped"));
        assert!(queues.take_head("s1", &row("stopped")).is_none());
        assert_eq!(queues.list("s1").as_array().unwrap().len(), 1);
        // Resumed: the fresh life's prompt takes the next entry.
        assert!(queues.take_head("s1", &row("unknown")).is_none());
        assert_eq!(queues.take_head("s1", &row("input")).unwrap().text, "two");
    }

    #[test]
    fn failure_pauses_until_any_mutation() {
        let queues = Queues::default();
        queues.add("s1", "one", None).unwrap();
        let head = queues.take_head("s1", &row("input")).unwrap();
        queues.restore("s1", 0, head, Some("rejected".into()));
        let enriched = queues.enrich(json!({"sessionId":"s1"}));
        assert_eq!(enriched["queuePaused"], true);
        assert_eq!(enriched["queueError"], "rejected");
        assert_eq!(enriched["queuedMessages"][0]["text"], "one");
        assert!(queues.take_head("s1", &row("input")).is_none(), "paused");
        let other = queues.add("s1", "two", None).unwrap();
        let enriched = queues.enrich(json!({"sessionId":"s1","queueError":"stale"}));
        assert_eq!(enriched["queuePaused"], false);
        assert!(enriched.get("queueError").is_none());
        assert_eq!(queues.take_head("s1", &row("input")).unwrap().text, "one");
        // An explicit send restores to its own index without pausing.
        queues.observe(&row("responding"));
        let (index, taken) = queues.take("s1", &other, Some(&row("input"))).unwrap();
        queues.restore("s1", index, taken, None);
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"}))["queuePaused"],
            false
        );
        assert_eq!(queues.list("s1")[0]["text"], "two");
    }

    #[test]
    fn rows_without_a_queue_are_untouched() {
        let queues = Queues::default();
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"})),
            json!({"sessionId":"s1"})
        );
        queues.add("s1", "one", None).unwrap();
        queues.clear("s1");
        assert_eq!(
            queues.enrich(json!({"sessionId":"s1"}))["queuedMessages"],
            json!([])
        );
        assert!(queues.forget("s1"));
        assert!(!queues.forget("s1"));
    }

    #[tokio::test]
    async fn persistence_round_trips_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("message-queues.json");
        let queues = Queues::open(path.clone());
        queues.add("keep", "one", Some(json!([{"a":1}]))).unwrap();
        queues.add("keep", "two", None).unwrap();
        let head = queues.take_head("keep", &row_for("keep", "input")).unwrap();
        queues.restore("keep", 0, head, Some("boom".into()));
        queues.add("drop", "x", None).unwrap();
        queues.add("empty", "x", None).unwrap();
        queues.clear("empty");
        queues.persist().await.unwrap();
        let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["version"], 1);
        assert!(raw["sessions"].get("empty").is_none(), "{raw}");
        let reopened = Queues::open(path.clone());
        assert_eq!(reopened.list("keep"), queues.list("keep"));
        let row = reopened.enrich(json!({"sessionId":"keep"}));
        assert_eq!(row["queuePaused"], true);
        assert_eq!(row["queueError"], "boom");
        reopened.retain(&["keep".to_owned()].into());
        assert_eq!(reopened.list("drop"), json!([]));
        reopened.persist().await.unwrap();
        let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(raw["sessions"].get("drop").is_none());
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(Queues::open(path).pending_sessions(), Vec::<String>::new());
    }

    #[test]
    fn an_in_flight_turn_across_restart_waits_for_the_stale_fallback() {
        let mut queue = Queue {
            entries: vec![Entry {
                id: "q-1".into(),
                text: "next".into(),
                created_at: 1,
                attachments: None,
            }],
            awaiting_turn: true,
            turn: Some(Turn::started(None)),
            ..Default::default()
        };
        queue.observe(&row("input"));
        assert!(!queue.wants_flush(&row("input")));
        queue.turn.as_mut().unwrap().since = Instant::now() - STALE_TURN;
        queue.observe(&row("input"));
        assert!(queue.wants_flush(&row("input")));
    }

    fn row_for(id: &str, mode: &str) -> Value {
        let mut value = row(mode);
        value["sessionId"] = json!(id);
        value["session_id"] = json!(id);
        value
    }
}
