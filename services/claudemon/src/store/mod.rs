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
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;
        upsert_session_tx(&tx, event)?;
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
        let mut stmt = guard.prepare(
            "SELECT id, cwd, tool_call_count, created_at, last_event_at
             FROM sessions ORDER BY last_event_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let cwd: String = r.get(1)?;
            Ok(RestoredSession {
                id: r.get(0)?,
                cwd: (!cwd.is_empty()).then_some(cwd),
                tool_calls: r.get::<_, i64>(2)?.max(0) as u64,
                created_at: r.get(3)?,
                last_event_at: r.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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
}

fn upsert_session_tx(tx: &rusqlite::Transaction<'_>, event: &HookEvent) -> Result<()> {
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
    let branch = payload.get("branch").and_then(Value::as_str);

    tx.execute(
        "INSERT INTO sessions (
            id, name, project, cwd, worktree_path, branch, base_branch, model,
            state, pid, created_at, last_event_at, total_cost_usd, tool_call_count
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 'working', NULL, ?8, ?8, 0, 0)
         ON CONFLICT(id) DO UPDATE SET
            last_event_at = excluded.last_event_at,
            cwd = CASE WHEN sessions.cwd = '' THEN excluded.cwd ELSE sessions.cwd END,
            model = COALESCE(sessions.model, excluded.model),
            branch = COALESCE(sessions.branch, excluded.branch)",
        params![
            session_id,
            name,
            project,
            cwd,
            worktree_path,
            branch,
            model,
            now
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
        let mut p = std::env::temp_dir();
        p.push(format!("claudemon-test-{}.db", uuid::Uuid::new_v4()));
        p
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
