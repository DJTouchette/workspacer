use std::collections::VecDeque;
use std::sync::Arc;

use dashmap::DashMap;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use time::OffsetDateTime;

use super::state::{HookEvent, Pending, SessionMode, SessionState, StatusLine};
use crate::protocol::WrapperMessage;

const BROADCAST_CAPACITY: usize = 256;
const HOOK_BROADCAST_CAPACITY: usize = 256;
const STATUS_BROADCAST_CAPACITY: usize = 256;
const OUTPUT_BUFFER_CAP: usize = 256 * 1024; // 256 KiB per session
const BYTE_BROADCAST_CAPACITY: usize = 1024;

/// Per-session ring buffer of raw PTY bytes the child has produced so far.
#[derive(Default)]
pub struct OutputBuffer {
    bytes: VecDeque<u8>,
    cap: usize,
}

impl OutputBuffer {
    fn new(cap: usize) -> Self {
        Self { bytes: VecDeque::with_capacity(cap.min(8192)), cap }
    }

    fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= self.cap {
            self.bytes.clear();
            let tail = &chunk[chunk.len() - self.cap..];
            self.bytes.extend(tail.iter().copied());
            return;
        }
        let overflow = (self.bytes.len() + chunk.len()).saturating_sub(self.cap);
        for _ in 0..overflow {
            self.bytes.pop_front();
        }
        self.bytes.extend(chunk.iter().copied());
    }

    fn snapshot(&self) -> Vec<u8> {
        self.bytes.iter().copied().collect()
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SessionUpdate {
    pub session_id: String,
    pub event: String,
    pub state: SessionState,
}

/// A statusLine tick for one session. Broadcast on its own channel (not the
/// hook fanout) because the statusLine command fires very frequently — routing
/// it through `hook_tx` would flood the SQLite persistence task.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StatusLineUpdate {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub status_line: StatusLine,
}

/// Handle the daemon keeps for each connected wrapper. Sending into `tx`
/// reaches the wrapper's WebSocket and ultimately the child's stdin / signals.
#[derive(Clone)]
pub struct WrapperHandle {
    pub tx: mpsc::UnboundedSender<WrapperMessage>,
}

#[derive(Clone)]
pub struct SessionStore {
    states: Arc<DashMap<String, SessionState>>,
    wrappers: Arc<DashMap<String, WrapperHandle>>,
    buffers: Arc<DashMap<String, Arc<Mutex<OutputBuffer>>>>,
    bytes_tx: Arc<DashMap<String, broadcast::Sender<Vec<u8>>>>,
    update_tx: broadcast::Sender<SessionUpdate>,
    /// Raw hook events fanout — every inbound HookEvent is broadcast here
    /// before state-machine processing, so clients that want the unaggregated
    /// stream (e.g. a richer external session store) can subscribe.
    hook_tx: broadcast::Sender<HookEvent>,
    /// StatusLine fanout — kept separate from `hook_tx` so the high-frequency
    /// statusLine ticks never reach the SQLite persistence task.
    status_tx: broadcast::Sender<StatusLineUpdate>,
    /// Per-session opt-in for the deferred-hook gateway. When `true`,
    /// PreToolUse hook responses are parked until a client decides
    /// (or until the daemon's timeout fires).
    gates: Arc<DashMap<String, bool>>,
    /// Currently-parked decision for a session, keyed by session_id.
    /// At most one is outstanding because Claude Code is blocked on it.
    decisions: Arc<DashMap<String, oneshot::Sender<Value>>>,
    /// Pending in-daemon spawns indexed by cwd: when claude's `SessionStart`
    /// hook arrives with a matching cwd, we rewrite the hook's session_id to
    /// the spawn's pre-assigned UUID and stash an alias so subsequent hook
    /// events resolve to the same entry.
    pending_spawns_by_cwd: Arc<DashMap<String, String>>,
    /// Alias map: claude's hook session_id → our canonical (spawn) session_id.
    aliases: Arc<DashMap<String, String>>,
}

impl SessionStore {
    pub fn new() -> Self {
        let (update_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (hook_tx, _) = broadcast::channel(HOOK_BROADCAST_CAPACITY);
        let (status_tx, _) = broadcast::channel(STATUS_BROADCAST_CAPACITY);
        Self {
            states: Arc::new(DashMap::new()),
            wrappers: Arc::new(DashMap::new()),
            buffers: Arc::new(DashMap::new()),
            bytes_tx: Arc::new(DashMap::new()),
            update_tx,
            hook_tx,
            status_tx,
            gates: Arc::new(DashMap::new()),
            decisions: Arc::new(DashMap::new()),
            pending_spawns_by_cwd: Arc::new(DashMap::new()),
            aliases: Arc::new(DashMap::new()),
        }
    }

    /// Repopulate the in-memory session list from persisted rows at startup,
    /// marking each as [`SessionMode::Stopped`]. The processes themselves are
    /// gone (they were the previous daemon's children), but the rows let clients
    /// see prior agents again and resume them — a respawn launches
    /// `claude --resume <id>`, and because we pin `--session-id` at spawn the row
    /// id doubles as claude's transcript uuid, so the conversation reopens rather
    /// than starting blank. A live entry (none exist at boot) always wins.
    pub fn hydrate(&self, sessions: Vec<crate::store::RestoredSession>) {
        for s in sessions {
            self.states.entry(s.id.clone()).or_insert_with(|| {
                let mut st = SessionState::new(s.id.clone(), s.cwd.clone());
                st.mode = SessionMode::Stopped;
                st.tool_calls = s.tool_calls;
                if let Ok(t) = OffsetDateTime::from_unix_timestamp(s.created_at) {
                    st.started_at = t;
                }
                if let Ok(t) = OffsetDateTime::from_unix_timestamp(s.last_event_at) {
                    st.updated_at = t;
                }
                st
            });
        }
    }

    // --- deferred-hook gateway ----------------------------------------------

    pub fn set_gate(&self, session_id: &str, on: bool) {
        if on {
            self.gates.insert(session_id.to_string(), true);
        } else {
            self.gates.remove(session_id);
            // If we're disabling the gate while a decision is parked, drop the
            // sender so the hook handler falls through to passthrough.
            self.decisions.remove(session_id);
        }
    }

    pub fn gate_enabled(&self, session_id: &str) -> bool {
        self.gates.get(session_id).map(|e| *e).unwrap_or(false)
    }

    /// Park a decision channel for this session and flip mode to Approval.
    /// Returns a receiver the caller awaits; another caller (typically
    /// `/decide` or `/approve`) resolves it via `resolve_decision`.
    pub fn park_decision(
        &self,
        session_id: &str,
        tool: Option<String>,
        raw: Value,
    ) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        // If there's already a pending decision, drop the old sender so
        // its waiter falls through. Shouldn't happen in practice because
        // Claude blocks on the hook, but keeps us safe under re-entrancy.
        self.decisions.insert(session_id.to_string(), tx);

        // Flip the observable state to Approval and surface the tool info
        // in `pending` so clients can render the right picker.
        let updated = {
            if let Some(mut state) = self.states.get_mut(session_id) {
                state.mode = SessionMode::Approval;
                let summary = raw
                    .get("tool_input")
                    .and_then(|ti| ti.get("command").or_else(|| ti.get("description")))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                state.pending = Some(Pending::Approval {
                    tool: tool.clone(),
                    summary,
                    raw,
                });
                Some(state.clone())
            } else {
                None
            }
        };
        if let Some(state) = updated {
            let _ = self.update_tx.send(SessionUpdate {
                session_id: session_id.to_string(),
                event: "PreToolUse".to_string(),
                state,
            });
        }
        rx
    }

    pub fn resolve_decision(&self, session_id: &str, decision: Value) -> bool {
        match self.decisions.remove(session_id) {
            Some((_, tx)) => tx.send(decision).is_ok(),
            None => false,
        }
    }

    pub fn has_pending_decision(&self, session_id: &str) -> bool {
        self.decisions.contains_key(session_id)
    }

    pub fn clear_pending_decision(&self, session_id: &str) {
        self.decisions.remove(session_id);
    }

    pub fn ingest(&self, mut event: HookEvent) -> SessionState {
        // Alias resolution: if claude's session_id has already been mapped to
        // our canonical (spawn-side) id, rewrite. For the first SessionStart of
        // a spawn, register the alias by looking up the pending spawn by cwd.
        if let Some(canonical) = self.aliases.get(&event.session_id).map(|e| e.clone()) {
            event.session_id = canonical;
        } else if !self.states.contains_key(&event.session_id) && event.event == "SessionStart" {
            // Only guess by cwd when we don't already know this id. When the
            // caller pinned `--session-id`, claude's hook id *is* our spawn id
            // (already in `states`), so we must skip the cwd guess — otherwise a
            // sibling spawn sharing the cwd could steal this session's hooks.
            if let Some(cwd) = event.cwd.clone() {
                if let Some((_, canonical)) = self.pending_spawns_by_cwd.remove(&cwd) {
                    self.aliases.insert(event.session_id.clone(), canonical.clone());
                    event.session_id = canonical;
                }
            }
        }

        // Broadcast the *post-aliasing* event so subscribers see the canonical
        // session_id Workspacer (and other clients) already know about.
        let _ = self.hook_tx.send(event.clone());

        let state = {
            let mut entry = self
                .states
                .entry(event.session_id.clone())
                .or_insert_with(|| SessionState::new(event.session_id.clone(), event.cwd.clone()));
            entry.apply(&event);
            entry.clone()
        };
        let _ = self.update_tx.send(SessionUpdate {
            session_id: event.session_id.clone(),
            event: event.event.clone(),
            state: state.clone(),
        });
        state
    }

    /// Apply a Claude Code statusLine payload to its session.
    ///
    /// The statusLine JSON carries Claude's *own* session id (same id its hooks
    /// use), so we resolve it through the same alias map `ingest` builds —
    /// landing on the canonical (spawn-side) id Workspacer knows. No-op if the
    /// session isn't registered yet: the statusLine command fires repeatedly,
    /// so the next tick lands once `SessionStart` has created the alias. Returns
    /// the updated state (and broadcasts a `StatusLine` update) when matched.
    pub fn ingest_status_line(&self, raw: &Value) -> Option<SessionState> {
        let sid = raw.get("session_id").and_then(Value::as_str)?;
        let canonical = self
            .aliases
            .get(sid)
            .map(|e| e.clone())
            .unwrap_or_else(|| sid.to_string());

        let status = StatusLine::from_claude_json(raw);
        let state = {
            let mut entry = self.states.get_mut(&canonical)?;
            let session = entry.value_mut();
            session.status_line = Some(status.clone());
            session.updated_at = OffsetDateTime::now_utc();
            session.clone()
        };
        let _ = self.status_tx.send(StatusLineUpdate {
            session_id: canonical,
            cwd: state.cwd.clone(),
            status_line: status,
        });
        Some(state)
    }

    pub fn list(&self) -> Vec<SessionState> {
        self.states.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get(&self, session_id: &str) -> Option<SessionState> {
        self.states.get(session_id).map(|e| e.clone())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionUpdate> {
        self.update_tx.subscribe()
    }

    pub fn subscribe_hooks(&self) -> broadcast::Receiver<HookEvent> {
        self.hook_tx.subscribe()
    }

    pub fn subscribe_status_lines(&self) -> broadcast::Receiver<StatusLineUpdate> {
        self.status_tx.subscribe()
    }

    // --- wrapper-driven session lifecycle -----------------------------------

    pub fn register_wrapper(
        &self,
        session_id: &str,
        cwd: &str,
        handle: WrapperHandle,
    ) -> SessionState {
        // Treat wrapper registration as a synthetic SessionStart so the state
        // machine produces the same observable behavior as hook-driven starts.
        let synthetic = HookEvent {
            event: "SessionStart".to_string(),
            session_id: session_id.to_string(),
            cwd: Some(cwd.to_string()),
            timestamp: None,
            payload: serde_json::Map::new(),
        };
        let state = self.ingest(synthetic);
        self.wrappers.insert(session_id.to_string(), handle);
        self.buffers
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(OutputBuffer::new(OUTPUT_BUFFER_CAP))));
        let (tx, _) = broadcast::channel(BYTE_BROADCAST_CAPACITY);
        self.bytes_tx.insert(session_id.to_string(), tx);
        state
    }

    pub fn deregister_wrapper(&self, session_id: &str) {
        self.wrappers.remove(session_id);
        self.buffers.remove(session_id);
        self.bytes_tx.remove(session_id);
        let synthetic = HookEvent {
            event: "SessionEnd".to_string(),
            session_id: session_id.to_string(),
            cwd: None,
            timestamp: None,
            payload: serde_json::Map::new(),
        };
        let _ = self.ingest(synthetic);
    }

    /// Register an in-daemon spawn before claude's SessionStart hook fires.
    /// The session is created upfront with our chosen session_id so clients
    /// can immediately subscribe to bytes, send input, etc. We also remember
    /// the cwd so that when claude's SessionStart arrives later we can alias
    /// claude's session_id to ours (see `ingest`).
    pub fn register_spawn(
        &self,
        session_id: &str,
        cwd: &str,
        handle: WrapperHandle,
    ) -> SessionState {
        let state = {
            let entry = self
                .states
                .entry(session_id.to_string())
                .or_insert_with(|| {
                    SessionState::new(session_id.to_string(), Some(cwd.to_string()))
                });
            entry.clone()
        };
        self.wrappers.insert(session_id.to_string(), handle);
        self.buffers
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(OutputBuffer::new(OUTPUT_BUFFER_CAP))));
        self.bytes_tx
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(BYTE_BROADCAST_CAPACITY).0);
        self.pending_spawns_by_cwd
            .insert(cwd.to_string(), session_id.to_string());
        let _ = self.update_tx.send(SessionUpdate {
            session_id: session_id.to_string(),
            event: "Spawn".to_string(),
            state: state.clone(),
        });
        state
    }

    /// Drop a previously-registered spawn that has not yet bound to a claude
    /// hook session. Used when /sessions/spawn fails after partial setup or
    /// the child exits before SessionStart fires.
    pub fn drop_pending_spawn(&self, session_id: &str, cwd: &str) {
        self.pending_spawns_by_cwd.remove(cwd);
        self.wrappers.remove(session_id);
        self.buffers.remove(session_id);
        self.bytes_tx.remove(session_id);
        self.states.remove(session_id);
    }

    pub fn wrapper(&self, session_id: &str) -> Option<WrapperHandle> {
        self.wrappers.get(session_id).map(|h| h.clone())
    }

    pub async fn record_output(&self, session_id: &str, chunk: &[u8]) {
        // Hold the buffer lock across both the ring-buffer push and the
        // broadcast send so a concurrent snapshot_and_subscribe can't see a
        // chunk in the snapshot *and* receive it again via the broadcast.
        let Some(buf) = self.buffers.get(session_id).map(|e| e.clone()) else {
            return;
        };
        let tx = self.bytes_tx.get(session_id).map(|e| e.clone());
        let mut guard = buf.lock().await;
        guard.push(chunk);
        if let Some(tx) = tx {
            let _ = tx.send(chunk.to_vec());
        }
    }

    pub async fn output_snapshot(&self, session_id: &str) -> Option<Vec<u8>> {
        let buf = self.buffers.get(session_id).map(|e| e.clone())?;
        let snapshot = buf.lock().await.snapshot();
        Some(snapshot)
    }

    pub fn subscribe_bytes(&self, session_id: &str) -> Option<broadcast::Receiver<Vec<u8>>> {
        self.bytes_tx.get(session_id).map(|e| e.subscribe())
    }

    /// Atomically take a snapshot of the ring buffer and subscribe to live
    /// bytes. The buffer mutex is held across both operations, and
    /// `record_output` holds the same mutex across its push+broadcast, so the
    /// returned snapshot and receiver are gap-free and duplicate-free: any
    /// chunk written before this call is in the snapshot only; any chunk
    /// written after is delivered via the receiver only.
    pub async fn snapshot_and_subscribe(
        &self,
        session_id: &str,
    ) -> Option<(Vec<u8>, broadcast::Receiver<Vec<u8>>)> {
        let buf = self.buffers.get(session_id).map(|e| e.clone())?;
        let tx = self.bytes_tx.get(session_id).map(|e| e.clone())?;
        let guard = buf.lock().await;
        let snapshot = guard.snapshot();
        let rx = tx.subscribe();
        drop(guard);
        Some((snapshot, rx))
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event: &str, session_id: &str, cwd: &str) -> HookEvent {
        HookEvent {
            event: event.into(),
            session_id: session_id.into(),
            cwd: Some(cwd.into()),
            timestamp: None,
            payload: serde_json::Map::new(),
        }
    }

    fn handle() -> WrapperHandle {
        let (tx, _rx) = mpsc::unbounded_channel();
        WrapperHandle { tx }
    }

    #[test]
    fn hydrate_restores_sessions_as_stopped_without_clobbering_live() {
        let store = SessionStore::new();
        // A live session already present before hydration (e.g. an early hook).
        store.ingest(hook("SessionStart", "live", "/work/live"));

        store.hydrate(vec![
            crate::store::RestoredSession {
                id: "restored".into(),
                cwd: Some("/work/restored".into()),
                tool_calls: 7,
                created_at: 1000,
                last_event_at: 2000,
            },
            // Same id as the live one — must NOT overwrite it back to stopped.
            crate::store::RestoredSession {
                id: "live".into(),
                cwd: Some("/work/live".into()),
                tool_calls: 0,
                created_at: 1,
                last_event_at: 2,
            },
        ]);

        let restored = store.get("restored").expect("restored session present");
        assert_eq!(restored.mode, SessionMode::Stopped);
        assert_eq!(restored.cwd.as_deref(), Some("/work/restored"));
        assert_eq!(restored.tool_calls, 7);

        let live = store.get("live").expect("live session present");
        assert_ne!(live.mode, SessionMode::Stopped, "live entry must win over hydrate");
    }

    // A pinned spawn (claude launched with `--session-id` == our id) must keep
    // its own hooks even when a sibling spawn shares the cwd. Without the
    // `states.contains_key` guard, SessionStart would consume the cwd's pending
    // entry and re-alias to the sibling, stealing the session — the root cause
    // of "wrong transcript" with several agents in one repo.
    #[test]
    fn pinned_session_id_not_stolen_by_cwd_sibling() {
        let store = SessionStore::new();
        let cwd = "/work/repo";
        // The later spawn overwrites pending_spawns_by_cwd[cwd].
        store.register_spawn("AAA", cwd, handle());
        store.register_spawn("BBB", cwd, handle());

        let state = store.ingest(hook("SessionStart", "AAA", cwd));
        assert_eq!(state.session_id, "AAA", "pinned hook must apply to its own state");
        assert!(store.get("AAA").is_some());
        assert!(!store.aliases.contains_key("AAA"), "pinned id must not be aliased away");
    }

    // Legacy path: a spawn with no pinned id (claude picks its own session id)
    // still correlates by cwd on the first SessionStart.
    #[test]
    fn legacy_unpinned_session_aliases_by_cwd() {
        let store = SessionStore::new();
        let cwd = "/work/solo";
        store.register_spawn("canonical-uuid", cwd, handle());

        let state = store.ingest(hook("SessionStart", "claude-own-id", cwd));
        assert_eq!(state.session_id, "canonical-uuid");
        assert_eq!(
            store.aliases.get("claude-own-id").map(|e| e.clone()),
            Some("canonical-uuid".to_string()),
        );
    }

    // A statusLine payload arrives with Claude's own session id; it must resolve
    // through the alias map to the canonical (spawn) id and land on that state.
    #[test]
    fn status_line_resolves_alias_and_lands_on_canonical_session() {
        let store = SessionStore::new();
        let cwd = "/work/repo";
        store.register_spawn("canonical-uuid", cwd, handle());
        // SessionStart binds claude's id → canonical via cwd.
        store.ingest(hook("SessionStart", "claude-own-id", cwd));

        let raw = serde_json::json!({
            "session_id": "claude-own-id",
            "workspace": { "current_dir": cwd },
            "model": { "display_name": "Opus 4.8 (1M context)" },
            "context_window": { "used_percentage": 22, "total_input_tokens": 220_000, "total_output_tokens": 700 },
            "cost": { "total_cost_usd": 3.34 },
            "rate_limits": {
                "five_hour": { "used_percentage": 1.0, "resets_at": 1_738_425_600i64 },
                "seven_day": { "used_percentage": 35.0 }
            }
        });
        let state = store.ingest_status_line(&raw).expect("should match canonical session");
        assert_eq!(state.session_id, "canonical-uuid");
        let sl = state.status_line.expect("status_line set");
        assert_eq!(sl.model_display.as_deref(), Some("Opus 4.8 (1M context)"));
        assert_eq!(sl.context_used_pct, Some(22.0));
        assert_eq!(sl.cost_usd, Some(3.34));
        assert_eq!(sl.five_hour_pct, Some(1.0));
        assert_eq!(sl.five_hour_resets_at, Some(1_738_425_600));
        assert_eq!(sl.seven_day_pct, Some(35.0));
        assert_eq!(sl.seven_day_resets_at, None);
    }

    // Before any SessionStart, a statusLine for an unknown id is a silent no-op
    // (it fires repeatedly, so the next tick lands once the session registers).
    #[test]
    fn status_line_for_unknown_session_is_noop() {
        let store = SessionStore::new();
        let raw = serde_json::json!({ "session_id": "nobody", "context_window": { "used_percentage": 5 } });
        assert!(store.ingest_status_line(&raw).is_none());
        assert!(store.get("nobody").is_none(), "must not create a phantom session");
    }
}
