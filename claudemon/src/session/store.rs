use std::collections::VecDeque;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{broadcast, mpsc, Mutex};

use super::state::{HookEvent, SessionState};
use crate::protocol::WrapperMessage;

const BROADCAST_CAPACITY: usize = 256;
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
}

impl SessionStore {
    pub fn new() -> Self {
        let (update_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            states: Arc::new(DashMap::new()),
            wrappers: Arc::new(DashMap::new()),
            buffers: Arc::new(DashMap::new()),
            bytes_tx: Arc::new(DashMap::new()),
            update_tx,
        }
    }

    pub fn ingest(&self, event: HookEvent) -> SessionState {
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
