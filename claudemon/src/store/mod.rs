//! SQLite-backed cold storage for sessions, events, items, and asks.
//!
//! Sits alongside the in-memory [`crate::session::SessionStore`]. The hot path
//! (hook intake, mode tracking, PTY bytes) keeps using the in-memory store
//! for latency; this module persists the event stream out-of-band so v2
//! features (inbox items, transcripts search, ask history) survive restarts.

pub mod items;
pub mod schema;

pub use items::{ItemAction, ItemRow, ListFilter};

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use serde_json::Value;
use time::OffsetDateTime;

use crate::classifier::{
    self, ItemAction as ClassifierAction, ItemKind, NewItem, OpenItem,
    Output as ClassifierOutput, RecentEvent, SessionSnapshot, SessionState,
};
use crate::session::HookEvent;

/// Window of past events the classifier inspects when deciding whether a
/// session is stuck in a tool-call loop. Matches `classifier::STUCK_WINDOW_EVENTS`
/// with headroom; the classifier itself caps how many it actually looks at.
const RECENT_EVENTS_FETCH: usize = 20;

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

    /// Persist the event, then run the v2 classifier and apply its actions.
    /// All writes happen in one transaction so a partial failure can't leave
    /// the inbox out of sync with the event log.
    pub fn record_and_classify(
        &self,
        event: &HookEvent,
        now_unix: i64,
    ) -> Result<ClassifyOutcome> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;

        upsert_session_tx(&tx, event)?;
        // Snoozed-on-event wake: any items waiting for the right trigger
        // come back to unread before the classifier runs, so duplicate-touch
        // logic sees them as already-open.
        let unsnoozed_item_ids = wake_snoozed_for_event_tx(&tx, &event.session_id, &event.event, now_unix)?;
        // Load context BEFORE inserting the new event so the classifier sees
        // only past events. Otherwise the "5x in 60s" rule double-counts the
        // incoming call.
        let session = load_session_snapshot_tx(&tx, &event.session_id)?;
        let recent_events = load_recent_events_tx(&tx, &event.session_id, RECENT_EVENTS_FETCH)?;
        let open_items = load_open_items_tx(&tx, &event.session_id)?;
        let event_row_id = insert_event_tx(&tx, event)?;

        let output = classifier::classify(classifier::Input {
            session,
            recent_events,
            open_items,
            event: event.clone(),
            event_row_id,
            now_unix,
        });

        let applied = apply_classifier_output_tx(
            &tx,
            &event.session_id,
            now_unix,
            &output,
        )?;
        tx.commit()?;

        Ok(ClassifyOutcome {
            event_row_id,
            new_session_state: output.new_session_state,
            created_item_ids: applied.created_item_ids,
            touched_item_ids: applied.touched_item_ids,
            resolved_item_ids: applied.resolved_item_ids,
            unsnoozed_item_ids,
        })
    }

    /// Sweep for working sessions that have been silent long enough to count
    /// as stuck (spec §11, IDLE_STUCK_SECONDS). One transaction per session
    /// that gets an item; sessions already flagged are no-ops.
    pub fn idle_sweep(&self, now_unix: i64) -> Result<Vec<IdleHit>> {
        let candidates: Vec<(String, i64)> = {
            let guard = self.conn.lock().expect("db mutex poisoned");
            let mut stmt = guard.prepare(
                "SELECT id, last_event_at FROM sessions
                 WHERE state = 'working' AND last_event_at < ?1",
            )?;
            let rows = stmt.query_map(
                params![now_unix - classifier::IDLE_STUCK_SECONDS],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut hits = Vec::new();
        for (session_id, last_event_at) in candidates {
            let mut guard = self.conn.lock().expect("db mutex poisoned");
            let tx = guard.transaction()?;
            let open_items = load_open_items_tx(&tx, &session_id)?;
            let snapshot = SessionSnapshot {
                state: SessionState::Working,
                last_event_at_unix: last_event_at,
            };
            if let Some(item) = classifier::classify_idle(&snapshot, &open_items, now_unix) {
                let item_id = insert_item_tx(&tx, &session_id, now_unix, &item)?;
                update_session_state_tx(&tx, &session_id, SessionState::Stuck, now_unix)?;
                tx.commit()?;
                hits.push(IdleHit { session_id, item_id });
            }
        }
        Ok(hits)
    }
}

#[derive(Debug, Clone)]
pub struct ClassifyOutcome {
    pub event_row_id: i64,
    pub new_session_state: SessionState,
    pub created_item_ids: Vec<String>,
    pub touched_item_ids: Vec<String>,
    /// Ids of items that transitioned to `resolved` as a side effect (the
    /// classifier's `ResolveAllForSession` action). Empty for events that
    /// don't trigger resolution.
    pub resolved_item_ids: Vec<String>,
    /// Ids of items that woke from `snoozed_on_event` because the matching
    /// trigger fired (e.g. snoozed-until-next-event resolving on any new
    /// event for the session).
    pub unsnoozed_item_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct IdleHit {
    pub session_id: String,
    pub item_id: String,
}

#[derive(Default)]
struct AppliedOutput {
    created_item_ids: Vec<String>,
    touched_item_ids: Vec<String>,
    resolved_item_ids: Vec<String>,
}

fn apply_classifier_output_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    now_unix: i64,
    output: &ClassifierOutput,
) -> Result<AppliedOutput> {
    update_session_state_tx(tx, session_id, output.new_session_state, now_unix)?;
    let mut applied = AppliedOutput::default();
    for action in &output.actions {
        match action {
            ClassifierAction::Create(item) => {
                let id = insert_item_tx(tx, session_id, now_unix, item)?;
                applied.created_item_ids.push(id);
            }
            ClassifierAction::Touch(id) => {
                tx.execute(
                    "UPDATE items SET updated_at = ?1
                     WHERE id = ?2 AND state IN ('unread', 'read')",
                    params![now_unix, id],
                )?;
                applied.touched_item_ids.push(id.clone());
            }
            ClassifierAction::ResolveAllForSession => {
                // Capture ids first so the broadcaster can emit per-id events.
                let mut stmt = tx.prepare(
                    "SELECT id FROM items WHERE session_id = ?1
                     AND state IN ('unread', 'read', 'snoozed')",
                )?;
                let ids: Vec<String> = stmt
                    .query_map(params![session_id], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                drop(stmt);
                tx.execute(
                    "UPDATE items SET state = 'resolved', resolved_at = ?1
                     WHERE session_id = ?2 AND state IN ('unread', 'read', 'snoozed')",
                    params![now_unix, session_id],
                )?;
                applied.resolved_item_ids.extend(ids);
            }
        }
    }
    Ok(applied)
}

fn update_session_state_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    state: SessionState,
    now_unix: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET state = ?1, last_event_at = ?2 WHERE id = ?3",
        params![state.as_str(), now_unix, session_id],
    )?;
    Ok(())
}

fn insert_item_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    now_unix: i64,
    item: &NewItem,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO items (
            id, session_id, state, priority, kind, summary, context_paragraph,
            next_action, triggering_event_id, created_at, updated_at,
            resolved_at, snoozed_until, snoozed_on_event, flagged
         ) VALUES (?1, ?2, 'unread', ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?8,
                   NULL, NULL, NULL, 0)",
        params![
            id,
            session_id,
            item.priority,
            item.kind.as_str(),
            item.summary,
            item.next_action,
            item.triggering_event_id,
            now_unix,
        ],
    )?;
    Ok(id)
}

/// Resolve any `snoozed_on_event` waits that match this event. Returns the
/// ids of items that got woken so the broadcaster can fire `item_changed`.
///
/// Trigger mapping:
///   - `next_event`   → wakes on any hook event for the session
///   - `session_done` → wakes on `Stop` or `SessionEnd`
///   - other values (e.g. `tests_pass`) are user-defined and ignored here
fn wake_snoozed_for_event_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    event_name: &str,
    now_unix: i64,
) -> Result<Vec<String>> {
    let mut matching: Vec<&'static str> = vec!["next_event"];
    if event_name == "Stop" || event_name == "SessionEnd" {
        matching.push("session_done");
    }
    let placeholders = matching
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(",");
    let select_sql = format!(
        "SELECT id FROM items WHERE session_id = ?1 AND state = 'snoozed'
         AND snoozed_on_event IN ({placeholders})"
    );
    let mut select = tx.prepare(&select_sql)?;
    let mut params_vec: Vec<rusqlite::types::Value> =
        vec![session_id.to_string().into()];
    for trigger in &matching {
        params_vec.push((*trigger).to_string().into());
    }
    let ids: Vec<String> = select
        .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(select);

    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let update_placeholders = (0..ids.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(",");
    let update_sql = format!(
        "UPDATE items SET state = 'unread',
            snoozed_until = NULL, snoozed_on_event = NULL, updated_at = ?1
         WHERE id IN ({update_placeholders})"
    );
    let mut params_vec: Vec<rusqlite::types::Value> = vec![now_unix.into()];
    for id in &ids {
        params_vec.push(id.clone().into());
    }
    tx.execute(&update_sql, rusqlite::params_from_iter(params_vec.iter()))?;
    Ok(ids)
}

fn load_session_snapshot_tx(tx: &Transaction<'_>, session_id: &str) -> Result<SessionSnapshot> {
    let (state_str, last_event_at): (String, i64) = tx.query_row(
        "SELECT state, last_event_at FROM sessions WHERE id = ?1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let state = SessionState::from_str(&state_str).unwrap_or(SessionState::Working);
    Ok(SessionSnapshot {
        state,
        last_event_at_unix: last_event_at,
    })
}

fn load_recent_events_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    limit: usize,
) -> Result<Vec<RecentEvent>> {
    let mut stmt = tx.prepare(
        "SELECT event_type, tool_name, timestamp, payload_json FROM events
         WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![session_id, limit as i64], |row| {
        let event_type: String = row.get(0)?;
        let tool_name: Option<String> = row.get(1)?;
        let timestamp: i64 = row.get(2)?;
        let payload_json: String = row.get(3)?;
        Ok((event_type, tool_name, timestamp, payload_json))
    })?;
    let mut events: Vec<RecentEvent> = rows
        .map(|r| {
            let (event_type, tool_name, timestamp, payload_json) = r?;
            let payload: Value =
                serde_json::from_str(&payload_json).unwrap_or(Value::Null);
            let tool_input = payload.get("tool_input");
            let tool_input_hash = classifier::canonical_tool_input_hash(tool_input);
            Ok::<_, anyhow::Error>(RecentEvent {
                event_type,
                tool_name,
                tool_input_hash,
                timestamp,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    // Caller wants chronological order (oldest first).
    events.reverse();
    Ok(events)
}

fn load_open_items_tx(tx: &Transaction<'_>, session_id: &str) -> Result<Vec<OpenItem>> {
    let mut stmt = tx.prepare(
        "SELECT id, kind, priority FROM items
         WHERE session_id = ?1 AND state IN ('unread', 'read')",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i32>(2)?,
        ))
    })?;
    let items = rows
        .filter_map(|r| r.ok())
        .filter_map(|(id, kind_str, priority)| {
            ItemKind::from_str(&kind_str).map(|kind| OpenItem { id, kind, priority })
        })
        .collect();
    Ok(items)
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
        params![session_id, name, project, cwd, worktree_path, branch, model, now],
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
            .query_row("SELECT COUNT(*) FROM events WHERE session_id = 's1'", [], |r| {
                r.get(0)
            })
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
    fn classify_permission_request_inserts_inbox_item() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_and_classify(&ev("SessionStart", "s3"), 1000).unwrap();

        let mut req = ev("PermissionRequest", "s3");
        req.payload.insert("tool_name".into(), json!("Bash"));
        let outcome = db.record_and_classify(&req, 1001).unwrap();
        assert_eq!(outcome.new_session_state, SessionState::NeedsInput);
        assert_eq!(outcome.created_item_ids.len(), 1);

        let conn = db.conn.lock().unwrap();
        let (kind, priority, state): (String, i32, String) = conn
            .query_row(
                "SELECT kind, priority, state FROM items WHERE session_id = 's3'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "needs_input");
        assert_eq!(priority, 95);
        assert_eq!(state, "unread");
        let session_state: String = conn
            .query_row("SELECT state FROM sessions WHERE id = 's3'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(session_state, "needs_input");
    }

    #[test]
    fn repeated_permission_request_touches_instead_of_duplicating() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_and_classify(&ev("SessionStart", "s4"), 1000).unwrap();
        let mut req = ev("PermissionRequest", "s4");
        req.payload.insert("tool_name".into(), json!("Bash"));
        db.record_and_classify(&req, 1001).unwrap();
        let second = db.record_and_classify(&req, 1002).unwrap();
        assert!(second.created_item_ids.is_empty());
        assert_eq!(second.touched_item_ids.len(), 1);
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM items WHERE session_id = 's4'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn session_end_resolves_open_items() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_and_classify(&ev("SessionStart", "s5"), 1000).unwrap();
        let mut req = ev("PermissionRequest", "s5");
        req.payload.insert("tool_name".into(), json!("Bash"));
        db.record_and_classify(&req, 1001).unwrap();
        db.record_and_classify(&ev("SessionEnd", "s5"), 1002).unwrap();

        let conn = db.conn.lock().unwrap();
        let (count_open, count_resolved): (i64, i64) = conn
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM items WHERE session_id='s5' AND state='unread'),
                    (SELECT COUNT(*) FROM items WHERE session_id='s5' AND state='resolved')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count_open, 0);
        assert_eq!(count_resolved, 1);
        let state: String = conn
            .query_row("SELECT state FROM sessions WHERE id = 's5'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "ended");
    }

    #[test]
    fn repeated_tool_use_promotes_to_stuck() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_and_classify(&ev("SessionStart", "s6"), 1000).unwrap();
        let mut pre = ev("PreToolUse", "s6");
        pre.payload.insert("tool_name".into(), json!("Bash"));
        pre.payload.insert("tool_input".into(), json!({"command": "ls"}));
        // First four don't trigger; the fifth flips to stuck.
        db.record_and_classify(&pre, 1001).unwrap();
        db.record_and_classify(&pre, 1002).unwrap();
        db.record_and_classify(&pre, 1003).unwrap();
        db.record_and_classify(&pre, 1004).unwrap();
        let fifth = db.record_and_classify(&pre, 1005).unwrap();
        assert_eq!(fifth.new_session_state, SessionState::Stuck);
        assert_eq!(fifth.created_item_ids.len(), 1);

        // A sixth identical call shouldn't pile on a second stuck item.
        let sixth = db.record_and_classify(&pre, 1006).unwrap();
        assert!(sixth.created_item_ids.is_empty());
    }

    #[test]
    fn idle_sweep_creates_stuck_item_for_silent_working_session() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_and_classify(&ev("SessionStart", "s7"), 1000).unwrap();
        let hits = db
            .idle_sweep(1000 + classifier::IDLE_STUCK_SECONDS + 1)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "s7");

        let (kind, priority): (String, i32) = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT kind, priority FROM items WHERE id = ?1",
                params![hits[0].item_id.clone()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "stuck");
        assert_eq!(priority, 60);
    }

    #[test]
    fn idle_sweep_is_noop_when_session_recently_active() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        db.record_and_classify(&ev("SessionStart", "s8"), 1000).unwrap();
        let hits = db.idle_sweep(1000 + 60).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn snooze_on_next_event_wakes_on_next_hook() {
        let db = Db::open(tempfile_path()).unwrap();
        db.record_and_classify(&ev("SessionStart", "s-snooze"), 1000).unwrap();
        let mut req = ev("PermissionRequest", "s-snooze");
        req.payload.insert("tool_name".into(), json!("Bash"));
        let outcome = db.record_and_classify(&req, 1001).unwrap();
        let item_id = outcome.created_item_ids[0].clone();

        use crate::store::items::ItemAction;
        db.apply_item_action(
            &item_id,
            &ItemAction::SnoozeOnEvent { on: "next_event".into() },
            1002,
        )
        .unwrap();
        assert_eq!(db.get_item(&item_id).unwrap().unwrap().state, "snoozed");

        let mut pre = ev("PreToolUse", "s-snooze");
        pre.payload.insert("tool_name".into(), json!("Edit"));
        let after = db.record_and_classify(&pre, 1003).unwrap();
        assert_eq!(after.unsnoozed_item_ids, vec![item_id.clone()]);
        assert_eq!(db.get_item(&item_id).unwrap().unwrap().state, "unread");
    }

    #[test]
    fn snooze_on_session_done_waits_for_stop() {
        let db = Db::open(tempfile_path()).unwrap();
        db.record_and_classify(&ev("SessionStart", "s-done"), 1000).unwrap();
        let mut req = ev("PermissionRequest", "s-done");
        req.payload.insert("tool_name".into(), json!("Bash"));
        let item_id = db
            .record_and_classify(&req, 1001)
            .unwrap()
            .created_item_ids[0]
            .clone();
        use crate::store::items::ItemAction;
        db.apply_item_action(
            &item_id,
            &ItemAction::SnoozeOnEvent { on: "session_done".into() },
            1002,
        )
        .unwrap();

        let mut pre = ev("PreToolUse", "s-done");
        pre.payload.insert("tool_name".into(), json!("Edit"));
        let after_pre = db.record_and_classify(&pre, 1003).unwrap();
        assert!(after_pre.unsnoozed_item_ids.is_empty());
        assert_eq!(db.get_item(&item_id).unwrap().unwrap().state, "snoozed");

        let after_stop = db.record_and_classify(&ev("Stop", "s-done"), 1004).unwrap();
        assert_eq!(after_stop.unsnoozed_item_ids, vec![item_id.clone()]);
        assert_eq!(db.get_item(&item_id).unwrap().unwrap().state, "unread");
    }

    #[test]
    fn snooze_on_event_with_unknown_trigger_stays_snoozed() {
        let db = Db::open(tempfile_path()).unwrap();
        db.record_and_classify(&ev("SessionStart", "s-tp"), 1000).unwrap();
        let mut req = ev("PermissionRequest", "s-tp");
        req.payload.insert("tool_name".into(), json!("Bash"));
        let item_id = db
            .record_and_classify(&req, 1001)
            .unwrap()
            .created_item_ids[0]
            .clone();
        use crate::store::items::ItemAction;
        db.apply_item_action(
            &item_id,
            &ItemAction::SnoozeOnEvent { on: "tests_pass".into() },
            1002,
        )
        .unwrap();
        let mut pre = ev("PreToolUse", "s-tp");
        pre.payload.insert("tool_name".into(), json!("Edit"));
        let after_pre = db.record_and_classify(&pre, 1003).unwrap();
        assert!(after_pre.unsnoozed_item_ids.is_empty());
        let after_stop = db.record_and_classify(&ev("Stop", "s-tp"), 1004).unwrap();
        assert!(after_stop.unsnoozed_item_ids.is_empty());
        assert_eq!(db.get_item(&item_id).unwrap().unwrap().state, "snoozed");
    }

    fn tempfile_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("claudemon-test-{}.db", uuid::Uuid::new_v4()));
        p
    }
}
