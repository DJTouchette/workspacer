//! The history recorder: writes every agent session into SQLite and keeps a
//! copy of its transcript (docs/session-roster.md §5).
//!
//! Hook persistence (`spawn_persistence_task`) only ever saw Claude hook
//! events, so stream and managed sessions had no row and vanished from History
//! once evicted from memory. This task follows the session-update stream
//! instead, which every provider drives:
//!
//! - A row is written on every mode change, and at most every
//!   [`RECORD_EVERY`] otherwise. A row is only *created* for a session seen in
//!   an agent mode, so terminal shells, which share the spawn path, never
//!   enter History.
//! - When a session stops, its transcript is copied. Every [`SWEEP_EVERY`],
//!   resident sessions are re-recorded and their transcripts re-copied when
//!   size or mtime changed. Copies run on their own worker so a large file
//!   never delays row writes.
//! - Broadcast lag is repaired by a sweep, not by replaying the missed updates.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast::error::RecvError, mpsc};

use crate::session::{SessionMode, SessionState, SessionStore};
use crate::store::history::{self, HistoryFacts};
use crate::store::Db;

/// Longest gap between two row writes for a session whose mode is unchanged.
pub(crate) const RECORD_EVERY: Duration = Duration::from_secs(30);
/// Re-record and re-copy cadence for resident sessions.
pub(crate) const SWEEP_EVERY: Duration = Duration::from_secs(5 * 60);
/// Queued copies beyond this are dropped; the next sweep catches them up.
const COPY_QUEUE: usize = 256;

pub(crate) fn spawn(store: SessionStore, db: Db) {
    let (copy_tx, copy_rx) = mpsc::channel(COPY_QUEUE);
    tokio::spawn(copy_worker(db.clone(), copy_rx));
    tokio::spawn(async move {
        Recorder::new(store, db, copy_tx).run().await;
    });
}

struct Recorder {
    store: SessionStore,
    db: Db,
    copies: mpsc::Sender<SessionState>,
    seen_agent: HashSet<String>,
    last: HashMap<String, (SessionMode, Instant)>,
}

impl Recorder {
    fn new(store: SessionStore, db: Db, copies: mpsc::Sender<SessionState>) -> Self {
        Self {
            store,
            db,
            copies,
            seen_agent: HashSet::new(),
            last: HashMap::new(),
        }
    }

    async fn run(mut self) {
        let mut updates = self.store.subscribe();
        let mut sweep = tokio::time::interval(SWEEP_EVERY);
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                update = updates.recv() => match update {
                    Ok(update) => {
                        if update.event == "SessionRemoved" {
                            // Failed spawns and deletes from History. A later
                            // life of the id starts from scratch.
                            self.seen_agent.remove(&update.session_id);
                            self.last.remove(&update.session_id);
                            continue;
                        }
                        self.observe(update.state).await;
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "history recorder lagged; sweeping");
                        self.sweep().await;
                    }
                    Err(RecvError::Closed) => break,
                },
                _ = sweep.tick() => self.sweep().await,
            }
        }
    }

    async fn observe(&mut self, state: SessionState) {
        let id = state.session_id.clone();
        if history::agent_mode(state.mode) {
            self.seen_agent.insert(id.clone());
        }
        let prior = self.last.get(&id).copied();
        let changed = prior.is_none_or(|(mode, _)| mode != state.mode);
        let due = prior.is_none_or(|(_, at)| at.elapsed() >= RECORD_EVERY);
        if !changed && !due {
            return;
        }
        self.last.insert(id, (state.mode, Instant::now()));
        let stopped = state.mode == SessionMode::Stopped;
        self.write(vec![state.clone()]).await;
        if changed && stopped {
            self.queue_copy(state);
        }
    }

    /// Re-record every resident session and queue a change-only copy for each.
    async fn sweep(&mut self) {
        let states = self.store.list();
        for state in &states {
            if history::agent_mode(state.mode) {
                self.seen_agent.insert(state.session_id.clone());
            }
            self.last
                .insert(state.session_id.clone(), (state.mode, Instant::now()));
        }
        self.write(states.clone()).await;
        for state in states {
            self.queue_copy(state);
        }
    }

    async fn write(&self, states: Vec<SessionState>) {
        if states.is_empty() {
            return;
        }
        let facts: Vec<_> = states
            .iter()
            .map(|s| {
                (
                    HistoryFacts::from_state(s),
                    self.seen_agent.contains(&s.session_id),
                )
            })
            .collect();
        let db = self.db.clone();
        match tokio::task::spawn_blocking(move || db.record_history(&facts)).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::warn!(?err, "recording session history failed"),
            Err(err) => tracing::warn!(?err, "history write task panicked"),
        }
    }

    fn queue_copy(&self, state: SessionState) {
        // Full queue: dropped here, retried by the next sweep.
        let _ = self.copies.try_send(state);
    }
}

async fn copy_worker(db: Db, mut rx: mpsc::Receiver<SessionState>) {
    while let Some(state) = rx.recv().await {
        if let Err(err) = copy_one(&db, &state).await {
            tracing::debug!(?err, session = %state.session_id, "transcript copy skipped");
        }
    }
}

/// The provider's transcript for a session, confined to where that provider
/// writes. `None` for providers without a file transcript.
pub(crate) async fn source_transcript(state: &SessionState) -> Option<PathBuf> {
    match state.provider.as_str() {
        "claude" => {
            let path = match state.transcript_path.clone() {
                Some(path) => path,
                None => crate::session::claude_subagents::discover_parent(state).await?,
            };
            let path = PathBuf::from(path);
            crate::session::transcript::path_is_allowed(&path).then_some(path)
        }
        "codex" => {
            let id = state.session_id.clone();
            tokio::task::spawn_blocking(move || {
                let thread = crate::providers::codex_rollout::thread_for(&id)?;
                let path = crate::providers::codex_rollout::rollout_for_thread(&thread)?;
                let home = crate::providers::codex_rollout::codex_home()?;
                path.starts_with(home.join("sessions")).then_some(path)
            })
            .await
            .ok()
            .flatten()
        }
        _ => None,
    }
}

async fn copy_one(db: &Db, state: &SessionState) -> anyhow::Result<()> {
    let Some(path) = source_transcript(state).await else {
        return Ok(());
    };
    let db = db.clone();
    let id = state.session_id.clone();
    let provider = state.provider.clone();
    let learned = state.transcript_path.is_none() && provider == "claude";
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if learned {
            db.note_transcript_path(&id, &path.to_string_lossy())?;
        }
        history::copy_transcript(&db, &id, &provider, &path)?;
        Ok(())
    })
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::conversation::tests::{transcript_fixture, two_turn_rows};
    use crate::session::{HookEvent, PendingWrite};

    async fn until<T>(mut probe: impl FnMut() -> Option<T>) -> T {
        for _ in 0..500 {
            if let Some(value) = probe() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never held");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agent_sessions_are_recorded_shells_are_not_and_stops_copy_the_transcript() {
        let store = SessionStore::new();
        let db = Db::open(crate::testtmp::db_path("recorder")).unwrap();
        spawn(store.clone(), db.clone());
        // Let the recorder subscribe before the first update.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // A terminal shell: a PTY spawn that never leaves Unknown, then exits.
        // It must not enter History.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        store.register_spawn("sh", "/w", crate::session::store::WrapperHandle { tx });
        let generation = store.claim_generation("sh");
        store.release_spawn("sh", "/w", generation);
        assert_eq!(store.get("sh").unwrap().mode, SessionMode::Stopped);

        // A managed Codex session working, then stopping.
        store.register_managed("m1", "/w", "codex");
        store.set_managed_mode("m1", SessionMode::Input, PendingWrite::Keep);
        let row = until(|| db.history_row("m1").unwrap()).await;
        assert_eq!(row.provider(), "codex");
        store.set_managed_mode("m1", SessionMode::Stopped, PendingWrite::Keep);
        until(|| db.history_row("m1").unwrap().and_then(|r| r.ended_at)).await;
        // Updates are processed in order, so "sh" was decided before "m1".
        assert!(db.history_row("sh").unwrap().is_none());

        // A Claude session with a transcript gets a copy when it stops.
        let (_dir, transcript) = transcript_fixture(&two_turn_rows());
        store.register_managed("c1", "/work", "claude");
        let mut start = HookEvent {
            event: "SessionStart".into(),
            session_id: "c1".into(),
            cwd: Some("/work".into()),
            timestamp: None,
            payload: serde_json::Map::new(),
        };
        start
            .payload
            .insert("transcript_path".into(), transcript.clone().into());
        store.ingest(start);
        store.set_managed_mode("c1", SessionMode::Input, PendingWrite::Keep);
        until(|| db.history_row("c1").unwrap()).await;
        store.set_managed_mode("c1", SessionMode::Stopped, PendingWrite::Keep);
        let copy = until(|| db.history_row("c1").unwrap().and_then(|r| r.copy)).await;
        assert_eq!(copy.original, transcript);
        assert_eq!(
            std::fs::read(&copy.copy).unwrap(),
            std::fs::read(&transcript).unwrap()
        );
    }
}
