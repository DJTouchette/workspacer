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
}
