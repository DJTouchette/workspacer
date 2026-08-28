//! SQLite-backed cold storage for sessions and their event stream.
//!
//! Sits alongside the in-memory [`crate::session::SessionStore`]. The hot path
//! (hook intake, mode tracking, PTY bytes) keeps using the in-memory store for
//! latency; this module persists the event stream out-of-band so sessions
//! survive a daemon restart — `load_recent_sessions` rehydrates the in-memory
//! list on boot so prior agents reappear as resumable.

pub mod schema;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;
use time::OffsetDateTime;

use crate::session::HookEvent;

/// Thread-safe handle to the daemon's SQLite database.
///
/// Cloning is cheap (`Arc<Mutex<Connection>>`). All writes serialize through
/// the mutex; SQLite itself is the bottleneck at high write rates, not lock
/// contention.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    path: PathBuf,
}

/// One keep-warm heartbeat (a warm ping run by `daemon::heartbeat`). Lives in
/// its own table so warms never mix with sessions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HeartbeatRow {
    pub id: i64,
    /// Epoch seconds when the ping ran.
    pub at: i64,
    pub ok: bool,
    /// Which account's window was warmed: 'claude' | 'codex'.
    pub provider: String,
    pub model: String,
    /// The new 5h window's reset (epoch seconds), when the CLI reported one.
    pub resets_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db parent dir {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("opening sqlite at {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&conn).context("running schema migrations")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    /// Persist one hook event plus a session upsert. Both happen in a single
    /// transaction so the events row never references a missing session.
    pub fn record_event(&self, event: &HookEvent) -> Result<i64> {
        self.record_event_with_requested_model(event, None)
    }

    /// [`record_event`](Self::record_event), plus the model this session was
    /// ASKED for, when the in-memory store knows one.
    ///
    /// It is threaded through here rather than written by a second statement
    /// because of an ordering hole: a spawn calls
    /// [`note_requested_model`](Self::note_requested_model) BEFORE the session
    /// has a row (rows are created by the first hook event), so that `UPDATE`
    /// matches nothing and the very next `INSERT` would leave the column NULL.
    /// Stamping it onto the row being created closes that, at no extra query —
    /// exactly how the neighbouring `model` column is handled.
    pub fn record_event_with_requested_model(
        &self,
        event: &HookEvent,
        requested_model: Option<&str>,
    ) -> Result<i64> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;
        upsert_session_tx(&tx, event, requested_model)?;
        let row_id = insert_event_tx(&tx, event)?;
        tx.commit()?;
        Ok(row_id)
    }

    /// Record one keep-warm heartbeat (see `daemon::heartbeat`). Returns the
    /// stored row with its assigned id.
    pub fn insert_heartbeat(&self, row: &HeartbeatRow) -> Result<HeartbeatRow> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        guard.execute(
            "INSERT INTO heartbeats (at, ok, provider, model, resets_at, duration_ms, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                row.at,
                row.ok as i64,
                row.provider,
                row.model,
                row.resets_at,
                row.duration_ms,
                row.error,
            ],
        )?;
        Ok(HeartbeatRow {
            id: guard.last_insert_rowid(),
            ..row.clone()
        })
    }

    /// The most recent heartbeats, newest first.
    pub fn list_heartbeats(&self, limit: usize) -> Result<Vec<HeartbeatRow>> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT id, at, ok, provider, model, resets_at, duration_ms, error
             FROM heartbeats ORDER BY at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                Ok(HeartbeatRow {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    ok: r.get::<_, i64>(2)? != 0,
                    provider: r.get(3)?,
                    model: r.get(4)?,
                    resets_at: r.get(5)?,
                    duration_ms: r.get(6)?,
                    error: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Load the most recently active persisted sessions so the daemon can
    /// repopulate its in-memory list after a restart. Ordered newest-first and
    /// capped at `limit`. Stale sessions are kept (never deleted) but the daemon
    /// marks them archived so they stay out of the default list while remaining
    /// resumable — see `SessionState::is_archived`.
    pub fn load_recent_sessions(&self, limit: usize) -> Result<Vec<RestoredSession>> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        // `user_prompt_count` is derived from the event log — the "did the user
        // actually talk to this agent" signal that lets the daemon hide
        // spawned-but-never-used sessions from the default list. A surviving
        // session row always keeps its events (prune deletes both together), so
        // the count is accurate for every row this returns.
        let mut stmt = guard.prepare(
            "SELECT s.id, s.cwd, s.tool_call_count, s.created_at, s.last_event_at,
                    (SELECT COUNT(*) FROM events e
                       WHERE e.session_id = s.id AND e.event_type = 'UserPromptSubmit'),
                    s.model, s.requested_model, s.transcript_path
             FROM sessions s ORDER BY s.last_event_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let cwd: String = r.get(1)?;
            Ok(RestoredSession {
                id: r.get(0)?,
                cwd: (!cwd.is_empty()).then_some(cwd),
                tool_calls: r.get::<_, i64>(2)?.max(0) as u64,
                created_at: r.get(3)?,
                last_event_at: r.get(4)?,
                user_prompt_count: r.get::<_, i64>(5)?.max(0) as u64,
                model: r.get::<_, Option<String>>(6)?.filter(|m| !m.is_empty()),
                requested_model: r.get::<_, Option<String>>(7)?.filter(|m| !m.is_empty()),
                transcript_path: r.get::<_, Option<String>>(8)?.filter(|p| !p.is_empty()),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Record the model a session was ASKED for, so a daemon restart rehydrates
    /// it (see [`RestoredSession::requested_model`]).
    ///
    /// A plain `UPDATE`: the row may not exist yet at spawn time — it is created
    /// by the first hook event — and inventing one here would list an agent
    /// that has not started. The other half of the write lives in
    /// [`upsert_session_tx`], which stamps the value onto the row it creates;
    /// between them every row that can ever be hydrated gets one.
    ///
    /// Best-effort by design, like the rest of this store's writes: a failure
    /// costs a context gauge after the next restart, not a session.
    pub fn note_requested_model(&self, session_id: &str, model: &str) {
        let model = model.trim();
        if model.is_empty() {
            return;
        }
        let Ok(guard) = self.conn.lock() else {
            return;
        };
        if let Err(err) = guard.execute(
            "UPDATE sessions SET requested_model = ?2 WHERE id = ?1",
            params![session_id, model],
        ) {
            tracing::warn!(session = %session_id, ?err, "persisting requested_model failed");
        }
    }

    /// Retention GC: delete sessions whose last event predates the 7-day archive
    /// window, cascading to their `events` rows in the same transaction. Always
    /// keeps the newest `keep` sessions regardless of age. Returns the count
    /// pruned.
    ///
    /// Conservative by construction. Sizing `keep` at `SESSION_HYDRATE_LIMIT`
    /// (100) matches exactly the window `load_recent_sessions` restores on the
    /// next boot, so nothing that would ever be hydrated is deleted — and a row
    /// older than the window would only come back *archived* (hidden) anyway.
    /// With fewer than `keep` rows, nothing is pruned.
    ///
    /// "Archived" is judged purely by age here: the daemon tracks live/Stopped
    /// mode only in the in-memory store and never stamps `state = 'stopped'` in
    /// SQLite, so a row idle past the window is by definition not a live
    /// session's recent activity.
    pub fn prune_archived(&self, keep: usize) -> Result<usize> {
        let cutoff = OffsetDateTime::now_utc().unix_timestamp()
            - crate::session::state::ARCHIVE_AFTER_SECONDS;
        self.prune_archived_before(keep, cutoff)
    }

    /// [`prune_archived`](Self::prune_archived) with an injected cutoff so tests
    /// can exercise retention without back-dating rows a real week.
    fn prune_archived_before(&self, keep: usize, cutoff: i64) -> Result<usize> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;
        // Freeze the prune set ONCE into a temp table, then delete its events
        // (child) and sessions (parent) from that frozen set. Re-evaluating the
        // `ORDER BY last_event_at DESC LIMIT keep` cut once per DELETE (as this
        // used to) can return a DIFFERENT arbitrary row among `last_event_at`
        // ties — the events-delete and sessions-delete then disagree, a session
        // is deleted while its events survive, and the FK (no ON DELETE CASCADE)
        // trips with "FOREIGN KEY constraint failed". The `id` tiebreak also
        // makes the "newest keep" cut deterministic across runs.
        tx.execute("DROP TABLE IF EXISTS _prune_ids", [])?;
        tx.execute(
            "CREATE TEMP TABLE _prune_ids AS \
             SELECT id FROM sessions \
             WHERE last_event_at < ?1 AND id NOT IN \
               (SELECT id FROM sessions ORDER BY last_event_at DESC, id DESC LIMIT ?2)",
            params![cutoff, keep as i64],
        )?;
        tx.execute(
            "DELETE FROM events WHERE session_id IN (SELECT id FROM _prune_ids)",
            [],
        )?;
        let pruned = tx.execute(
            "DELETE FROM sessions WHERE id IN (SELECT id FROM _prune_ids)",
            [],
        )?;
        tx.execute("DROP TABLE _prune_ids", [])?;
        tx.commit()?;
        Ok(pruned)
    }
}

/// A persisted session row, restored into the in-memory store on daemon boot.
pub struct RestoredSession {
    pub id: String,
    pub cwd: Option<String>,
    pub tool_calls: u64,
    pub created_at: i64,
    pub last_event_at: i64,
    /// Number of `UserPromptSubmit` events on record for this session — 0 means
    /// it was spawned but never actually prompted (see `is_empty_stopped`).
    pub user_prompt_count: u64,
    /// The concrete model the transcript reported, when one was recorded.
    pub model: Option<String>,
    /// The model this session was ASKED for. `hydrate` used to drop both of
    /// these on the floor — the columns existed (well, `model` did) and were
    /// simply not read — so a restarted daemon rebuilt a 1M session with no
    /// idea it was one, and every client's gauge read ~5× too full for the rest
    /// of its life. `None` is honest for a session the daemon did not spawn.
    pub requested_model: Option<String>,
    /// The transcript this session's usage is folded from. THE load-bearing
    /// field for cost-at-boot: `usage::usage_for_path(None)` is all zeros, so a
    /// row rehydrated without this reports $0.00 and 0 tokens no matter how much
    /// the session actually cost. `None` only for a session whose event log
    /// never carried a `transcript_path` — which is honestly unknown, not zero.
    pub transcript_path: Option<String>,
}

fn upsert_session_tx(
    tx: &rusqlite::Transaction<'_>,
    event: &HookEvent,
    requested_model: Option<&str>,
) -> Result<()> {
    let now = event_timestamp_unix(event);
    let session_id = &event.session_id;
    let cwd = event.cwd.as_deref().unwrap_or("");
    // Sessions are append-on-first-event; subsequent events only bump
    // last_event_at and tool_call_count (when applicable).
    let payload = &event.payload;
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| derive_name_from_cwd(cwd));
    let project = payload
        .get("project")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| name.clone());
    let worktree_path = payload
        .get("worktree_path")
        .and_then(Value::as_str)
        .unwrap_or(cwd);
    let model = payload.get("model").and_then(Value::as_str);
    // The model the session was ASKED for, which the transcript can never
    // report: Claude Code strips the `[1m]` marker from the id it writes.
    let requested_model = requested_model.map(str::trim).filter(|m| !m.is_empty());
    let branch = payload.get("branch").and_then(Value::as_str);
    // The transcript this session's usage folds from. Every Claude Code hook
    // carries it; persisting it here is what lets a rehydrated row report real
    // cost instead of $0.00 (see `schema::add_session_transcript_path`).
    //
    // Stored EXACTLY as the CLI spelled it — never canonicalized. The path
    // string is also the only carrier of which account wrote the session, and
    // `~/.claude/accounts/<name>/projects` is a symlink to the shared
    // `~/.claude/projects`, so a `realpath` here would silently merge two
    // accounts into one.
    let transcript_path = payload
        .get("transcript_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty());

    tx.execute(
        "INSERT INTO sessions (
            id, name, project, cwd, worktree_path, branch, base_branch, model,
            state, pid, created_at, last_event_at, tool_call_count,
            requested_model, transcript_path
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 'working', NULL, ?8, ?8, 0, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            last_event_at = excluded.last_event_at,
            cwd = CASE WHEN sessions.cwd = '' THEN excluded.cwd ELSE sessions.cwd END,
            model = COALESCE(sessions.model, excluded.model),
            branch = COALESCE(sessions.branch, excluded.branch),
            requested_model = COALESCE(sessions.requested_model, excluded.requested_model),
            transcript_path =
                COALESCE(excluded.transcript_path, sessions.transcript_path)",
        params![
            session_id,
            name,
            project,
            cwd,
            worktree_path,
            branch,
            model,
            now,
            requested_model,
            transcript_path
        ],
    )?;

    if event.event == "PreToolUse" {
        tx.execute(
            "UPDATE sessions SET tool_call_count = tool_call_count + 1 WHERE id = ?1",
            params![session_id],
        )?;
    }
    Ok(())
}

fn insert_event_tx(tx: &rusqlite::Transaction<'_>, event: &HookEvent) -> Result<i64> {
    let ts = event_timestamp_unix(event);
    let tool_name = event
        .payload
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let payload_json = serde_json::to_string(&event.payload)?;
    tx.execute(
        "INSERT INTO events (session_id, timestamp, event_type, tool_name, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![event.session_id, ts, event.event, tool_name, payload_json],
    )?;
    Ok(tx.last_insert_rowid())
}

fn event_timestamp_unix(event: &HookEvent) -> i64 {
    event
        .timestamp
        .unwrap_or_else(OffsetDateTime::now_utc)
        .unix_timestamp()
}

fn derive_name_from_cwd(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string()
}

/// Default location for the SQLite file, honoring XDG_DATA_HOME when set
/// and falling back to `~/.claudemon/state.db`.
pub fn default_db_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("claudemon").join("state.db");
    }
    if let Some(home) = directories::BaseDirs::new() {
        return home.home_dir().join(".claudemon").join("state.db");
    }
    PathBuf::from(".claudemon/state.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(event: &str, session_id: &str) -> HookEvent {
        HookEvent {
            event: event.to_string(),
            session_id: session_id.to_string(),
            cwd: Some("/tmp/proj".to_string()),
            timestamp: None,
            payload: serde_json::Map::new(),
        }
    }

    fn tempfile_path() -> PathBuf {
        crate::testtmp::db_path("store-test")
    }

    /// THE OTHER daemon-restart drop, and the one every $0.00 came from.
    ///
    /// `usage::usage_for_session` folds from `state.transcript_path`, and
    /// `usage_for_path(None)` is `Usage::default()` — all zeros. The path was
    /// only ever set from a live hook, so before v6 every rehydrated row came
    /// back with `None` and reported no cost, no tokens and no model. Measured
    /// on the live daemon: 94 of 102 listed sessions.
    #[test]
    fn transcript_path_survives_a_daemon_restart() {
        let db = Db::open(tempfile_path()).unwrap();
        let mut e = ev("SessionStart", "s1");
        e.payload.insert(
            "transcript_path".into(),
            json!("/home/u/.claude/projects/-home-u-work/s1.jsonl"),
        );
        db.record_event(&e).unwrap();

        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(
            restored[0].transcript_path.as_deref(),
            Some("/home/u/.claude/projects/-home-u-work/s1.jsonl"),
            "without this the fold has nothing to read and bills $0.00",
        );
    }

    /// A later hook naming a DIFFERENT transcript wins; a later hook naming
    /// none must not blank the column (most hooks carry it, but the upsert has
    /// to survive one that does not).
    #[test]
    fn transcript_path_takes_the_newest_and_is_never_blanked() {
        let db = Db::open(tempfile_path()).unwrap();
        let mut first = ev("SessionStart", "s1");
        first
            .payload
            .insert("transcript_path".into(), json!("/home/u/.claude/projects/p/a.jsonl"));
        db.record_event(&first).unwrap();

        let mut moved = ev("PreToolUse", "s1");
        moved.payload.insert(
            "transcript_path".into(),
            json!("/home/u/.claude/accounts/work/projects/p/a.jsonl"),
        );
        db.record_event(&moved).unwrap();
        // No transcript_path at all on this one.
        db.record_event(&ev("Stop", "s1")).unwrap();

        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(
            restored[0].transcript_path.as_deref(),
            Some("/home/u/.claude/accounts/work/projects/p/a.jsonl"),
            "newest non-null wins, and a path-less hook leaves it alone",
        );
    }

    /// The v6 backfill, against the shape of a real pre-v6 database: session
    /// rows with no `transcript_path`, but an event log that has been recording
    /// the paths verbatim all along. This is recovery of stored data, not a
    /// guess — which is why 806 historical rows can be attributed and folded.
    #[test]
    fn migration_backfills_transcript_path_from_the_event_log() {
        let path = tempfile_path();
        {
            // Stand up a v5-era database by hand: no transcript_path column.
            let conn = Connection::open(&path).unwrap();
            schema::migrate(&conn).unwrap();
            conn.execute("ALTER TABLE sessions DROP COLUMN transcript_path", [])
                .unwrap();
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN total_cost_usd REAL DEFAULT 0",
                [],
            )
            .unwrap();
            conn.execute_batch(
                "DELETE FROM events; DELETE FROM sessions;
                 INSERT INTO sessions (id, name, project, cwd, worktree_path, state,
                                       created_at, last_event_at, tool_call_count)
                   VALUES ('old', 'n', 'p', '/w', '/w', 'working', 100, 200, 0);
                 INSERT INTO events (session_id, timestamp, event_type, payload_json)
                   VALUES ('old', 100, 'SessionStart',
                           '{\"transcript_path\":\"/home/u/.claude/projects/p/old.jsonl\"}'),
                          ('old', 150, 'PreToolUse', '{\"tool_name\":\"Bash\"}'),
                          ('old', 190, 'Stop',
                           '{\"transcript_path\":\"/home/u/.claude/projects/p/old.jsonl\"}');",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 5).unwrap();
        }

        let db = Db::open(&path).unwrap();
        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(
            restored[0].transcript_path.as_deref(),
            Some("/home/u/.claude/projects/p/old.jsonl"),
            "an existing row recovers its path from its own persisted events",
        );

        // …and the dead cost column is gone rather than left as a permanent 0.
        let guard = db.conn.lock().unwrap();
        let has_cost: bool = guard
            .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1")
            .unwrap()
            .exists(["total_cost_usd"])
            .unwrap();
        assert!(!has_cost, "total_cost_usd dropped, not left summing to 0.0");
    }

    /// A session whose event log genuinely never carried a path stays `None`.
    /// Unknown is not zero, and the backfill must never invent one.
    #[test]
    fn migration_leaves_a_pathless_session_null() {
        let path = tempfile_path();
        {
            let conn = Connection::open(&path).unwrap();
            schema::migrate(&conn).unwrap();
            conn.execute("ALTER TABLE sessions DROP COLUMN transcript_path", [])
                .unwrap();
            conn.execute_batch(
                "DELETE FROM events; DELETE FROM sessions;
                 INSERT INTO sessions (id, name, project, cwd, worktree_path, state,
                                       created_at, last_event_at, tool_call_count)
                   VALUES ('bare', 'n', 'p', '/w', '/w', 'working', 100, 200, 0);
                 INSERT INTO events (session_id, timestamp, event_type, payload_json)
                   VALUES ('bare', 100, 'SessionStart', '{}');",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 5).unwrap();
        }
        let db = Db::open(&path).unwrap();
        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(restored[0].transcript_path, None);
    }

    /// THE DAEMON-RESTART DROP, end to end.
    ///
    /// A 1M session survives a restart as a 1M session. Before this,
    /// `RestoredSession` carried neither model, `hydrate` read neither column,
    /// and the `requested_model` column did not exist — so a resumed
    /// `opus[1m]` agent came back as whatever the window table says about its
    /// marker-stripped transcript id (200k), and every gauge read ~5x too full
    /// for the rest of its life.
    ///
    /// The ORDERING is the substance of this test: the spawn records the model
    /// before the session has a row at all (rows are born from the first hook
    /// event), so the plain `UPDATE` matches nothing. It is
    /// `record_event_with_requested_model` stamping the value onto the row it
    /// creates that saves it.
    #[test]
    fn requested_model_survives_a_daemon_restart() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();

        // 1. Spawn records it. No row exists yet — this UPDATE hits nothing,
        //    and that is exactly the hole the second write closes.
        db.note_requested_model("s1", "opus[1m]");
        let restored = db.load_recent_sessions(10).unwrap();
        assert!(
            restored.is_empty(),
            "no row yet: the spawn-time UPDATE has nothing to update"
        );

        // 2. The first hook event creates the row, carrying the model with it.
        db.record_event_with_requested_model(&ev("SessionStart", "s1"), Some("opus[1m]"))
            .unwrap();

        // 3. Restart: this is what `hydrate` gets handed.
        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].requested_model.as_deref(), Some("opus[1m]"));
    }

    /// A session the daemon did not spawn has no requested model, and says so.
    /// `None`, not an empty string and not a guess — an adopted or foreign
    /// session genuinely does not know, and the resolver's job is to fall
    /// through to the contract table rather than be handed a fake marker.
    #[test]
    fn a_session_with_no_recorded_request_restores_as_unknown() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_event(&ev("SessionStart", "adopted")).unwrap();
        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].requested_model, None);
    }

    /// A later event must not blank a recorded request. The upsert takes
    /// `COALESCE(sessions.requested_model, excluded.requested_model)` for the
    /// same reason the `model` column beside it does: the in-memory store can
    /// be evicted (`evict_stale_stopped`) while the session's events keep
    /// arriving, and the very next hook would otherwise overwrite a real
    /// recorded model with the NULL of "nobody asked me this time".
    #[test]
    fn a_later_event_without_a_request_does_not_blank_it() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_event_with_requested_model(&ev("SessionStart", "s1"), Some("sonnet[1m]"))
            .unwrap();
        db.record_event(&ev("PreToolUse", "s1")).unwrap();
        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(restored[0].requested_model.as_deref(), Some("sonnet[1m]"));
    }

    #[test]
    fn heartbeats_round_trip_newest_first() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        let row = |at: i64, ok: bool| HeartbeatRow {
            id: 0,
            at,
            ok,
            provider: if ok { "claude".into() } else { "codex".into() },
            model: "haiku".into(),
            resets_at: ok.then_some(at + 5 * 3600),
            duration_ms: Some(1200),
            error: (!ok).then(|| "spawn failed".into()),
        };
        let first = db.insert_heartbeat(&row(1000, true)).unwrap();
        assert!(first.id > 0);
        db.insert_heartbeat(&row(2000, false)).unwrap();

        let all = db.list_heartbeats(10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].at, 2000); // newest first
        assert!(!all[0].ok);
        assert_eq!(all[0].provider, "codex");
        assert_eq!(all[0].error.as_deref(), Some("spawn failed"));
        assert_eq!(all[1].resets_at, Some(1000 + 5 * 3600));

        assert_eq!(db.list_heartbeats(1).unwrap().len(), 1);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn records_event_and_session() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        let id = db.record_event(&ev("SessionStart", "s1")).unwrap();
        assert!(id > 0);
        let conn = db.conn.lock().unwrap();
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(session_count, 1);
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE session_id = 's1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 1);
    }

    #[test]
    fn pre_tool_use_bumps_tool_count() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_event(&ev("SessionStart", "s2")).unwrap();
        let mut e = ev("PreToolUse", "s2");
        e.payload.insert("tool_name".into(), json!("Bash"));
        db.record_event(&e).unwrap();
        db.record_event(&e).unwrap();
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT tool_call_count FROM sessions WHERE id = 's2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn load_recent_sessions_is_newest_first_and_capped() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        // Insert three sessions with increasing last_event_at via SessionStart
        // (carries its own timestamp).
        for (i, sid) in ["old", "mid", "new"].iter().enumerate() {
            let mut e = ev("SessionStart", sid);
            e.timestamp = OffsetDateTime::from_unix_timestamp(1000 + i as i64 * 10).ok();
            db.record_event(&e).unwrap();
        }
        let all = db.load_recent_sessions(10).unwrap();
        assert_eq!(
            all.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["new", "mid", "old"],
            "newest last_event_at first"
        );
        assert_eq!(all[0].cwd.as_deref(), Some("/tmp/proj"));

        let capped = db.load_recent_sessions(2).unwrap();
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].id, "new");
    }

    #[test]
    fn load_recent_sessions_counts_user_prompts() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        // "used" gets two prompts; "empty" is spawned but never prompted.
        db.record_event(&ev("SessionStart", "used")).unwrap();
        db.record_event(&ev("UserPromptSubmit", "used")).unwrap();
        db.record_event(&ev("UserPromptSubmit", "used")).unwrap();
        db.record_event(&ev("SessionStart", "empty")).unwrap();

        let rows = db.load_recent_sessions(10).unwrap();
        let count = |id: &str| {
            rows.iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("{id} missing"))
                .user_prompt_count
        };
        assert_eq!(count("used"), 2, "both prompts counted");
        assert_eq!(
            count("empty"),
            0,
            "spawned-but-unused session has no prompts"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn prune_archived_deletes_old_sessions_and_their_events() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        // Five old sessions (t≈1000) and three recent (t≈9000), each with an
        // event carrying its own timestamp.
        for i in 0..5 {
            let mut e = ev("SessionStart", &format!("old{i}"));
            e.timestamp = OffsetDateTime::from_unix_timestamp(1000 + i as i64).ok();
            db.record_event(&e).unwrap();
        }
        for i in 0..3 {
            let mut e = ev("SessionStart", &format!("new{i}"));
            e.timestamp = OffsetDateTime::from_unix_timestamp(9000 + i as i64).ok();
            db.record_event(&e).unwrap();
        }

        // cutoff = 5000: all old rows predate it; keep the newest 2 overall
        // (new2, new1). The old rows are older than cutoff AND outside the
        // newest-2, so all five prune. Recent rows all survive (newer than cutoff).
        let pruned = db.prune_archived_before(2, 5000).unwrap();
        assert_eq!(pruned, 5, "all five old sessions pruned");

        let conn = db.conn.lock().unwrap();
        let sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 3, "the three recent sessions survive");
        let old_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE session_id LIKE 'old%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_events, 0, "pruned sessions' events cascaded away");
        let new_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE session_id LIKE 'new%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(new_events, 3, "surviving sessions keep their events");
    }

    #[test]
    fn prune_archived_keep_floor_protects_old_rows_and_nothing_recent() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        for i in 0..5 {
            let mut e = ev("SessionStart", &format!("old{i}"));
            e.timestamp = OffsetDateTime::from_unix_timestamp(1000 + i as i64).ok();
            db.record_event(&e).unwrap();
        }
        for i in 0..3 {
            let mut e = ev("SessionStart", &format!("new{i}"));
            e.timestamp = OffsetDateTime::from_unix_timestamp(9000 + i as i64).ok();
            db.record_event(&e).unwrap();
        }

        // keep = 6, cutoff = 5000. Newest 6 by last_event_at = new2,new1,new0,
        // old4,old3,old2 → those are protected even though old4/3/2 predate the
        // cutoff. Only old1 and old0 are both old AND outside the newest-6.
        let pruned = db.prune_archived_before(6, 5000).unwrap();
        assert_eq!(
            pruned, 2,
            "only the two oldest rows outside the keep floor go"
        );
        let conn = db.conn.lock().unwrap();
        for still in ["old2", "old3", "old4", "new0", "new1", "new2"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                    [still],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "{still} must be protected by the keep floor");
        }
        drop(conn);

        // A keep larger than the row count prunes nothing.
        assert_eq!(db.prune_archived_before(1000, 5000).unwrap(), 0);
    }

    #[test]
    fn prune_archived_survives_last_event_at_ties() {
        // Regression for the Windows "FOREIGN KEY constraint failed" (SQLite
        // 787): when many sessions share one last_event_at, the keep-boundary
        // falls inside a tie group. The old code re-evaluated the newest-`keep`
        // cut once per DELETE, so the events-delete and sessions-delete could
        // pick different tied rows — deleting a session while keeping its events
        // and tripping the FK. Freezing the prune set once must make this safe.
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        // Ten sessions ALL stamped at the same time, each with its own event.
        for i in 0..10 {
            let mut e = ev("SessionStart", &format!("tie{i}"));
            e.timestamp = OffsetDateTime::from_unix_timestamp(1000).ok();
            db.record_event(&e).unwrap();
        }

        // keep = 3 lands inside the tie group; cutoff = 5000 ages every row in.
        // Must not error, and must leave no event orphaned.
        let pruned = db.prune_archived_before(3, 5000).unwrap();
        assert_eq!(pruned, 7, "ten tied rows minus the keep-3 floor");

        let conn = db.conn.lock().unwrap();
        let sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 3, "exactly the keep floor survives");
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE session_id NOT IN (SELECT id FROM sessions)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0, "no event left pointing at a deleted session");
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 3, "surviving sessions keep their event");
    }
}
