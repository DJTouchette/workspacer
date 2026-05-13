use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::broadcast;

use super::state::{HookEvent, SessionState};

const BROADCAST_CAPACITY: usize = 256;

#[derive(Clone, Debug, serde::Serialize)]
pub struct SessionUpdate {
    pub session_id: String,
    pub event: String,
    pub state: SessionState,
}

#[derive(Clone)]
pub struct SessionStore {
    inner: Arc<DashMap<String, SessionState>>,
    tx: broadcast::Sender<SessionUpdate>,
}

impl SessionStore {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(DashMap::new()),
            tx,
        }
    }

    pub fn ingest(&self, event: HookEvent) -> SessionState {
        let state = {
            let mut entry = self
                .inner
                .entry(event.session_id.clone())
                .or_insert_with(|| SessionState::new(event.session_id.clone(), event.cwd.clone()));
            entry.apply(&event);
            entry.clone()
        };

        let _ = self.tx.send(SessionUpdate {
            session_id: event.session_id.clone(),
            event: event.event.clone(),
            state: state.clone(),
        });

        state
    }

    pub fn list(&self) -> Vec<SessionState> {
        self.inner.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get(&self, session_id: &str) -> Option<SessionState> {
        self.inner.get(session_id).map(|e| e.clone())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionUpdate> {
        self.tx.subscribe()
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}
