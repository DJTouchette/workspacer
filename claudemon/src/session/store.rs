use std::collections::VecDeque;
use std::sync::Arc;

use dashmap::DashMap;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use super::state::{HookEvent, Pending, SessionMode, SessionState};
use crate::protocol::WrapperMessage;

const BROADCAST_CAPACITY: usize = 256;
const HOOK_BROADCAST_CAPACITY: usize = 256;
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
        Self {
            states: Arc::new(DashMap::new()),
            wrappers: Arc::new(DashMap::new()),
            buffers: Arc::new(DashMap::new()),
            bytes_tx: Arc::new(DashMap::new()),
            update_tx,
            hook_tx,
            gates: Arc::new(DashMap::new()),
            decisions: Arc::new(DashMap::new()),
            pending_spawns_by_cwd: Arc::new(DashMap::new()),
            aliases: Arc::new(DashMap::new()),
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
        } else if event.event == "SessionStart" {
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
            let mut entry = self
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
        let (tx, _) = broadcast::channel(BYTE_BROADCAST_CAPACITY);
        self.bytes_tx.insert(session_id.to_string(), tx);
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
        if let Some(buf) = self.buffers.get(session_id).map(|e| e.clone()) {
            buf.lock().await.push(chunk);
        }
        if let Some(tx) = self.bytes_tx.get(session_id).map(|e| e.clone()) {
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
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}
