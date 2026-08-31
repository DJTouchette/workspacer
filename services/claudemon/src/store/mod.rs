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

use crate::session::{
    windows::{restore_persisted_model_selection, ModelSelection, PersistedModelSelection},
    HookEvent,
};

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
        self.record_event_with_spawn_facts(event, SpawnFacts::default())
    }

    /// [`record_event`](Self::record_event), plus the facts only the SPAWN
    /// knows — the model this session was asked for and the account it bills
    /// against. Neither can be recovered from the event itself.
    ///
    /// They are threaded through here rather than written by a second statement
    /// because of an ordering hole: a spawn records them in memory BEFORE the
    /// session has a row (rows are created by the first hook event), so an
    /// `UPDATE` would match nothing and the very next `INSERT` would leave the
    /// columns NULL. Stamping them onto the row being created closes that, at
    /// no extra query — exactly how the neighbouring `model` column is handled.
    pub fn record_event_with_spawn_facts(
        &self,
        event: &HookEvent,
        facts: SpawnFacts<'_>,
    ) -> Result<i64> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;
        upsert_session_tx(&tx, event, facts)?;
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
                    s.model, s.requested_model, s.transcript_path, s.config_root,
                    s.requested_model_identity, s.requested_context_window
             FROM sessions s ORDER BY s.last_event_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let cwd: String = r.get(1)?;
            let requested_model = r.get::<_, Option<String>>(7)?.filter(|m| !m.is_empty());
            let (requested_model_identity, canonical_identity_valid) = match r.get_ref(10)? {
                rusqlite::types::ValueRef::Null => (None, true),
                rusqlite::types::ValueRef::Text(value) => (
                    std::str::from_utf8(value)
                        .ok()
                        .filter(|model| !model.is_empty())
                        .map(str::to_owned),
                    true,
                ),
                _ => (None, false),
            };
            // SQLite is dynamically typed even for an INTEGER-affinity column;
            // a hand-edited/future row can hold text here. Treat that as invalid
            // canonical evidence and fall back instead of failing the entire
            // restore query (which would discard every row from daemon boot).
            let (requested_context_window, canonical_window_valid) = match r.get_ref(11)? {
                rusqlite::types::ValueRef::Null => (None, true),
                rusqlite::types::ValueRef::Integer(value) => (Some(value), true),
                _ => (None, false),
            };
            let requested_selection = restore_persisted_model_selection(
                (canonical_identity_valid && canonical_window_valid)
                    .then_some(requested_model_identity.as_deref())
                    .flatten(),
                requested_context_window,
                requested_model.as_deref(),
            );
            Ok(RestoredSession {
                id: r.get(0)?,
                cwd: (!cwd.is_empty()).then_some(cwd),
                tool_calls: r.get::<_, i64>(2)?.max(0) as u64,
                created_at: r.get(3)?,
                last_event_at: r.get(4)?,
                user_prompt_count: r.get::<_, i64>(5)?.max(0) as u64,
                model: r.get::<_, Option<String>>(6)?.filter(|m| !m.is_empty()),
                requested_model,
                requested_selection,
                transcript_path: r.get::<_, Option<String>>(8)?.filter(|p| !p.is_empty()),
                // No `filter(!is_empty)`: `Some("")` is the default account,
                // which is a different fact from `None` (unknown).
                config_root: r.get::<_, Option<String>>(9)?,
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
    pub fn note_requested_model_selection(
        &self,
        session_id: &str,
        persisted: &PersistedModelSelection,
    ) {
        let Ok(guard) = self.conn.lock() else {
            return;
        };
        if let Err(err) = guard.execute(
            "UPDATE sessions
                SET requested_model = ?2,
                    requested_model_identity = ?3,
                    requested_context_window = ?4
              WHERE id = ?1",
            params![
                session_id,
                persisted.legacy_model,
                persisted.selection.model,
                persisted.selection.context_window.map(|value| value as i64),
            ],
        ) {
            tracing::warn!(session = %session_id, ?err, "persisting requested model selection failed");
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
    /// Canonical selection recovered from the new columns when valid, otherwise
    /// lazily parsed from `requested_model`. Invalid/partial data never rejects
    /// the surrounding session row.
    pub requested_selection: Option<ModelSelection>,
    /// The transcript this session's usage is folded from. THE load-bearing
    /// field for cost-at-boot: `usage::usage_for_path(None)` is all zeros, so a
    /// row rehydrated without this reports $0.00 and 0 tokens no matter how much
    /// the session actually cost. `None` only for a session whose event log
    /// never carried a `transcript_path` — which is honestly unknown, not zero.
    pub transcript_path: Option<String>,
    /// The Claude account this session billed against, as recorded at spawn.
    /// `Some("")` is the DEFAULT account — a real answer. `None` is genuinely
    /// unknown: a row written before schema v8, or a session the daemon did
    /// not spawn. The two must not be conflated; see `schema::SCHEMA_V8`.
    pub config_root: Option<String>,
}

/// The facts about a session that only its SPAWN knows, carried into the
/// statement that creates its row. Both are `Option` because "the daemon did
/// not spawn this session" is a real state and must not be written as a value.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnFacts<'a> {
    /// Canonical + legacy pair, held together so row creation cannot write only
    /// one side of the compatibility contract.
    pub requested_selection: Option<&'a PersistedModelSelection>,
    /// The Claude config root, normalized — `Some("")` = the default account.
    pub config_root: Option<&'a str>,
}

fn upsert_session_tx(
    tx: &rusqlite::Transaction<'_>,
    event: &HookEvent,
    facts: SpawnFacts<'_>,
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
    let requested_model = facts
        .requested_selection
        .map(|selection| selection.legacy_model.as_str());
    let requested_model_identity = facts
        .requested_selection
        .map(|selection| selection.selection.model.as_str());
    let requested_context_window = facts
        .requested_selection
        .and_then(|selection| selection.selection.context_window)
        .map(|value| value as i64);
    // The account this session bills against. NOT `filter(!is_empty)` — the
    // empty string is the DEFAULT account, a known answer, and dropping it to
    // NULL would relabel every default-account session "unknown".
    let config_root = facts.config_root.map(str::trim);
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
            requested_model, transcript_path, config_root,
            requested_model_identity, requested_context_window
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 'working', NULL, ?8, ?8, 0,
                   ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET
            last_event_at = excluded.last_event_at,
            cwd = CASE WHEN sessions.cwd = '' THEN excluded.cwd ELSE sessions.cwd END,
            model = COALESCE(sessions.model, excluded.model),
            branch = COALESCE(sessions.branch, excluded.branch),
            -- Selection is one logical value. A supplied identity means all
            -- three columns (including a NULL context window) replace the prior
            -- request together; absent spawn facts leave all three untouched.
            requested_model = CASE
                WHEN excluded.requested_model_identity IS NOT NULL
                THEN excluded.requested_model ELSE sessions.requested_model END,
            requested_model_identity = CASE
                WHEN excluded.requested_model_identity IS NOT NULL
                THEN excluded.requested_model_identity ELSE sessions.requested_model_identity END,
            requested_context_window = CASE
                WHEN excluded.requested_model_identity IS NOT NULL
                THEN excluded.requested_context_window ELSE sessions.requested_context_window END,
            transcript_path =
                COALESCE(excluded.transcript_path, sessions.transcript_path),
            -- First non-NULL wins and then sticks. A session cannot change
            -- account mid-life, and later events carry no attribution at all,
            -- so a plain assignment would blank the spawn's answer on event 2.
            config_root = COALESCE(sessions.config_root, excluded.config_root)",
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
            transcript_path,
            config_root,
            requested_model_identity,
            requested_context_window
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

    fn selection(model: &str) -> PersistedModelSelection {
        crate::session::windows::normalize_persisted_model_selection(model).unwrap()
    }

    fn raw_selection_columns(
        db: &Db,
        session_id: &str,
    ) -> (Option<String>, Option<String>, Option<i64>) {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT requested_model, requested_model_identity, requested_context_window
                   FROM sessions WHERE id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    /// Test-side contract guard: a persisted selection is valid only when both
    /// the canonical source and the legacy compatibility projection exist and
    /// describe the same normalized request.
    fn dual_write_guard(db: &Db, session_id: &str) -> Result<(), String> {
        let (legacy, identity, context_window) = raw_selection_columns(db, session_id);
        let legacy = legacy.ok_or_else(|| "legacy requested_model is missing".to_string())?;
        let identity =
            identity.ok_or_else(|| "canonical requested_model_identity is missing".to_string())?;
        if legacy == identity {
            return Ok(());
        }
        let canonical = restore_persisted_model_selection(Some(&identity), context_window, None)
            .ok_or_else(|| "canonical selection is invalid".to_string())?;
        let expected = PersistedModelSelection::from_selection(canonical);
        if expected.legacy_model != legacy {
            return Err(format!(
                "legacy/canonical conflict: legacy={legacy:?}, canonical={expected:?}"
            ));
        }
        Ok(())
    }

    #[test]
    fn new_rows_dual_write_selectable_and_native_one_million_models() {
        let db = Db::open(tempfile_path()).unwrap();
        for (id, requested, legacy, identity, window) in [
            ("opus", "opus[1m]", "opus[1m]", "opus", Some(1_000_000)),
            (
                "sonnet",
                "sonnet-1m",
                "sonnet[1m]",
                "sonnet",
                Some(1_000_000),
            ),
            ("fable", "fable", "fable", "fable", None),
            ("mythos", "mythos", "mythos", "mythos", None),
        ] {
            let persisted = selection(requested);
            db.record_event_with_spawn_facts(
                &ev("SessionStart", id),
                SpawnFacts {
                    requested_selection: Some(&persisted),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                raw_selection_columns(&db, id),
                (Some(legacy.into()), Some(identity.into()), window),
                "{requested} must retain the right rollback representation",
            );
            dual_write_guard(&db, id).unwrap();
        }
    }

    #[test]
    fn persisted_model_selections_round_trip_without_losing_native_one_million() {
        struct Case {
            id: &'static str,
            requested: &'static str,
            legacy: &'static str,
            identity: &'static str,
            context_window: Option<i64>,
            requested_window: Option<u64>,
        }

        // Keep this at the SQLite boundary. The selection parser alone cannot
        // catch an INSERT/restore mismatch that loses an explicit native-1M
        // canonical window while deriving its intentionally bare legacy value.
        let cases = [
            Case {
                id: "fable-bare",
                requested: "fable",
                legacy: "fable",
                identity: "fable",
                context_window: None,
                requested_window: Some(1_000_000),
            },
            Case {
                id: "fable-legacy-marked",
                requested: "fable[1m]",
                legacy: "fable",
                identity: "fable",
                context_window: Some(1_000_000),
                requested_window: Some(1_000_000),
            },
            Case {
                id: "mythos-bare",
                requested: "mythos",
                legacy: "mythos",
                identity: "mythos",
                context_window: None,
                requested_window: Some(1_000_000),
            },
            Case {
                id: "mythos-legacy-marked",
                requested: "mythos-1m",
                legacy: "mythos",
                identity: "mythos",
                context_window: Some(1_000_000),
                requested_window: Some(1_000_000),
            },
            Case {
                id: "opus-1m",
                requested: "opus[1m]",
                legacy: "opus[1m]",
                identity: "opus",
                context_window: Some(1_000_000),
                requested_window: Some(1_000_000),
            },
            Case {
                id: "sonnet-1m",
                requested: "sonnet-1m",
                legacy: "sonnet[1m]",
                identity: "sonnet",
                context_window: Some(1_000_000),
                requested_window: Some(1_000_000),
            },
            Case {
                id: "opus-200k",
                requested: "opus",
                legacy: "opus",
                identity: "opus",
                context_window: Some(200_000),
                requested_window: None,
            },
            Case {
                id: "sonnet-200k",
                requested: "sonnet",
                legacy: "sonnet",
                identity: "sonnet",
                context_window: Some(200_000),
                requested_window: None,
            },
            Case {
                id: "codex",
                requested: "gpt-5-codex",
                legacy: "gpt-5-codex",
                identity: "gpt-5-codex",
                context_window: None,
                requested_window: None,
            },
            Case {
                id: "local",
                requested: "acme-local-model",
                legacy: "acme-local-model",
                identity: "acme-local-model",
                context_window: None,
                requested_window: None,
            },
        ];
        let path = tempfile_path();
        {
            let db = Db::open(&path).unwrap();
            for case in &cases {
                let persisted = PersistedModelSelection::from_selection(
                    crate::session::windows::normalize_model_selection(
                        case.requested,
                        case.context_window.map(|window| window as u64),
                    )
                    .unwrap(),
                );
                db.record_event_with_spawn_facts(
                    &ev("SessionStart", case.id),
                    SpawnFacts {
                        requested_selection: Some(&persisted),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    raw_selection_columns(&db, case.id),
                    (
                        Some(case.legacy.into()),
                        Some(case.identity.into()),
                        case.context_window,
                    ),
                    "{} canonical columns",
                    case.id,
                );
            }
        }

        let db = Db::open(&path).unwrap();
        let restored: std::collections::HashMap<_, _> = db
            .load_recent_sessions(100)
            .unwrap()
            .into_iter()
            .map(|row| (row.id, row.requested_selection))
            .collect();
        for case in &cases {
            let selection = restored[case.id]
                .as_ref()
                .expect("persisted selection restores");
            assert_eq!(selection.model, case.identity, "{} identity", case.id);
            assert_eq!(
                selection.context_window.map(|window| window as i64),
                case.context_window,
                "{} canonical context window",
                case.id,
            );
            assert_eq!(
                crate::session::windows::requested_window_for_selection(selection),
                case.requested_window,
                "{} retains the inherent/requested window after reopen",
                case.id,
            );
        }
    }

    #[test]
    fn opaque_non_claude_dash_one_m_round_trips_with_an_explicit_window() {
        let path = tempfile_path();
        let db = Db::open(&path).unwrap();
        let persisted = PersistedModelSelection {
            selection: ModelSelection {
                model: "vendor/custom-1m".into(),
                context_window: Some(1_000_000),
            },
            legacy_model: "vendor/custom-1m".into(),
        };
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "opaque-1m"),
            SpawnFacts {
                requested_selection: Some(&persisted),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            raw_selection_columns(&db, "opaque-1m"),
            (
                Some("vendor/custom-1m".into()),
                Some("vendor/custom-1m".into()),
                Some(1_000_000),
            )
        );
        dual_write_guard(&db, "opaque-1m").unwrap();

        let restored = db.load_recent_sessions(1).unwrap().remove(0);
        assert_eq!(
            restored.requested_model.as_deref(),
            Some("vendor/custom-1m")
        );
        assert_eq!(restored.requested_selection, Some(persisted.selection));
    }

    #[test]
    fn native_one_million_canonical_pairs_match_legacy_projections_and_heal() {
        let path = tempfile_path();
        {
            let db = Db::open(&path).unwrap();
            for (id, legacy_marked) in [("fable", "fable[1m]"), ("mythos", "mythos-1m")] {
                let persisted = selection(legacy_marked);
                db.record_event_with_spawn_facts(
                    &ev("SessionStart", id),
                    SpawnFacts {
                        requested_selection: Some(&persisted),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        }

        let db = Db::open(&path).unwrap();
        let restored = db.load_recent_sessions(10).unwrap();
        for row in &restored {
            assert_eq!(
                row.requested_selection,
                Some(ModelSelection {
                    model: row.id.clone(),
                    context_window: Some(1_000_000),
                }),
                "{} keeps the canonical native-1M pair authoritative",
                row.id,
            );
        }

        let store = crate::session::SessionStore::new();
        store.hydrate(restored);
        for id in ["fable", "mythos"] {
            assert_eq!(
                store.get(id).and_then(|state| state.requested_model),
                Some(id.into()),
                "public snapshot uses the same normalized spelling the heal will write",
            );
            let healed = store.requested_model_selection(id).unwrap();
            db.record_event_with_spawn_facts(
                &ev("PreToolUse", id),
                SpawnFacts {
                    requested_selection: Some(&healed),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                raw_selection_columns(&db, id),
                (Some(id.into()), Some(id.into()), Some(1_000_000)),
                "{} healed without losing its canonical native 1M window",
                id,
            );
        }

        drop(db);
        let reopened = Db::open(&path).unwrap();
        for row in reopened.load_recent_sessions(10).unwrap() {
            assert_eq!(
                row.requested_selection,
                Some(ModelSelection {
                    model: row.id.clone(),
                    context_window: Some(1_000_000),
                }),
                "{} remains canonical after the heal and another reopen",
                row.id,
            );
        }
    }

    #[test]
    fn old_legacy_rows_restore_lazily_without_backfill() {
        let db = Db::open(tempfile_path()).unwrap();
        db.record_event(&ev("SessionStart", "old")).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET requested_model = 'opus[1m]' WHERE id = 'old'",
                [],
            )
            .unwrap();

        let restored = db.load_recent_sessions(10).unwrap().remove(0);
        assert_eq!(restored.requested_model.as_deref(), Some("opus[1m]"));
        assert_eq!(
            restored.requested_selection,
            Some(ModelSelection {
                model: "opus".into(),
                context_window: Some(1_000_000),
            })
        );
        assert_eq!(
            raw_selection_columns(&db, "old"),
            (Some("opus[1m]".into()), None, None),
            "read compatibility is lazy and does not backfill the row",
        );
    }

    #[test]
    fn new_database_reopens_idempotently_and_legacy_projection_is_rollback_readable() {
        let path = tempfile_path();
        {
            let db = Db::open(&path).unwrap();
            let persisted = selection("opus[1m]");
            db.record_event_with_spawn_facts(
                &ev("SessionStart", "rollback"),
                SpawnFacts {
                    requested_selection: Some(&persisted),
                    ..Default::default()
                },
            )
            .unwrap();
        }

        let reopened = Db::open(&path).unwrap();
        assert_eq!(
            reopened.load_recent_sessions(10).unwrap()[0].requested_selection,
            Some(ModelSelection {
                model: "opus".into(),
                context_window: Some(1_000_000),
            }),
        );
        drop(reopened);

        // This is the prior schema consumer's projection: it knows only the v8
        // stamp and requested_model, and ignores additive nullable columns.
        let legacy = Connection::open(&path).unwrap();
        let version: i32 = legacy
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let requested: String = legacy
            .query_row(
                "SELECT requested_model FROM sessions WHERE id = 'rollback'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 8);
        assert_eq!(requested, "opus[1m]");
    }

    #[test]
    fn rollback_legacy_wins_conflicts_while_matching_or_invalid_data_falls_back_safely() {
        let db = Db::open(tempfile_path()).unwrap();
        for id in [
            "conflict",
            "matching",
            "partial",
            "invalid",
            "null-context-conflict",
            "canonical-only",
        ] {
            db.record_event(&ev("SessionStart", id)).unwrap();
        }
        let guard = db.conn.lock().unwrap();
        guard
            .execute(
                "UPDATE sessions SET requested_model = 'sonnet-1m',
                 requested_model_identity = 'sonnet', requested_context_window = 1000000
               WHERE id = 'matching'",
                [],
            )
            .unwrap();
        guard
            .execute(
                "UPDATE sessions SET requested_model = 'sonnet',
                 requested_model_identity = 'opus', requested_context_window = 1000000
               WHERE id = 'conflict'",
                [],
            )
            .unwrap();
        guard
            .execute(
                "UPDATE sessions SET requested_model = 'sonnet[1m]',
                 requested_context_window = 200000
               WHERE id = 'partial'",
                [],
            )
            .unwrap();
        guard
            .execute(
                "UPDATE sessions SET requested_model = 'fable',
                 requested_model_identity = 'opus', requested_context_window = 'wide'
               WHERE id = 'invalid'",
                [],
            )
            .unwrap();
        guard
            .execute(
                "UPDATE sessions SET requested_model = 'sonnet[1m]',
                 requested_model_identity = 'opus', requested_context_window = NULL
               WHERE id = 'null-context-conflict'",
                [],
            )
            .unwrap();
        guard
            .execute(
                "UPDATE sessions SET requested_model = NULL,
                 requested_model_identity = 'opus', requested_context_window = NULL
               WHERE id = 'canonical-only'",
                [],
            )
            .unwrap();
        drop(guard);

        let restored: std::collections::HashMap<_, _> = db
            .load_recent_sessions(10)
            .unwrap()
            .into_iter()
            .map(|row| (row.id, row.requested_selection))
            .collect();
        assert_eq!(restored.len(), 6, "bad selection data never drops its row");
        assert_eq!(
            restored["matching"],
            Some(ModelSelection {
                model: "sonnet".into(),
                context_window: Some(1_000_000),
            }),
            "matching normalized canonical and legacy values keep the canonical answer",
        );
        assert_eq!(
            restored["conflict"],
            Some(ModelSelection {
                model: "sonnet".into(),
                context_window: None,
            }),
            "a valid legacy disagreement is newer evidence from a v8 rollback writer",
        );
        assert_eq!(
            restored["partial"],
            Some(ModelSelection {
                model: "sonnet".into(),
                context_window: Some(1_000_000),
            }),
            "a context without an identity is partial and falls back",
        );
        assert_eq!(
            restored["invalid"],
            Some(ModelSelection {
                model: "fable".into(),
                context_window: None,
            }),
            "a non-integer canonical window is invalid and falls back without failing restore",
        );
        assert_eq!(
            restored["null-context-conflict"],
            Some(ModelSelection {
                model: "sonnet".into(),
                context_window: Some(1_000_000),
            }),
            "NULL context is valid canonical evidence, but a legacy disagreement is still newer",
        );
        assert_eq!(
            restored["canonical-only"],
            Some(ModelSelection {
                model: "opus".into(),
                context_window: None,
            }),
            "valid canonical data remains authoritative when legacy evidence is absent",
        );
    }

    /// The real rollback sequence, with the old side using the exact SQL from
    /// v8's `Db::note_requested_model`: new dual-write -> old read/switch -> new
    /// reopen -> next event. The event must heal all three columns rather than
    /// carrying the raw legacy value forward and preserving divergence.
    #[test]
    fn v8_legacy_only_switch_wins_then_the_next_new_daemon_write_heals_the_row() {
        let path = tempfile_path();
        let initial = selection("opus[1m]");
        {
            let db = Db::open(&path).unwrap();
            db.record_event_with_spawn_facts(
                &ev("SessionStart", "rollback-switch"),
                SpawnFacts {
                    requested_selection: Some(&initial),
                    ..Default::default()
                },
            )
            .unwrap();
            dual_write_guard(&db, "rollback-switch").unwrap();
        }

        // Prior v8 daemon: it accepts the unchanged version, reads only the
        // compatibility column, then a live /model switch updates only that
        // column. Keep this statement identical to the v8 implementation.
        {
            let old = Connection::open(&path).unwrap();
            let version: i32 = old
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            let before: String = old
                .query_row(
                    "SELECT requested_model FROM sessions WHERE id = 'rollback-switch'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(version, 8, "the v8 rollback window remains open");
            assert_eq!(before, "opus[1m]");
            old.execute(
                "UPDATE sessions SET requested_model = ?2 WHERE id = ?1",
                params!["rollback-switch", "sonnet-1m"],
            )
            .unwrap();
        }

        let reopened = Db::open(&path).unwrap();
        let restored = reopened.load_recent_sessions(10).unwrap();
        assert_eq!(
            restored[0].requested_selection,
            Some(ModelSelection {
                model: "sonnet".into(),
                context_window: Some(1_000_000),
            }),
            "the legacy-only v8 switch is newer than the stale canonical pair",
        );

        // This is the production path after restart: hydrate canonical memory,
        // read SpawnFacts from SessionStore, then persist the next hook event.
        let store = crate::session::SessionStore::new();
        store.hydrate(restored);
        assert_eq!(
            store
                .get("rollback-switch")
                .and_then(|state| state.requested_model),
            Some("sonnet[1m]".into()),
            "the public snapshot normalizes the v8 spelling before the next write heals SQLite",
        );
        let healed = store
            .requested_model_selection("rollback-switch")
            .expect("restored selection is carried into persistence");
        assert_eq!(
            healed.legacy_model, "sonnet[1m]",
            "normal writers derive the compatibility spelling instead of preserving raw v8 input",
        );
        reopened
            .record_event_with_spawn_facts(
                &ev("PreToolUse", "rollback-switch"),
                SpawnFacts {
                    requested_selection: Some(&healed),
                    ..Default::default()
                },
            )
            .unwrap();
        dual_write_guard(&reopened, "rollback-switch").unwrap();
        drop(reopened);

        // The healed projection is still a valid v8 value on a subsequent
        // rollback; the compatibility write never strands the older daemon.
        let old = Connection::open(&path).unwrap();
        let requested: String = old
            .query_row(
                "SELECT requested_model FROM sessions WHERE id = 'rollback-switch'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(requested, "sonnet[1m]");
    }

    #[test]
    fn concurrent_db_opens_create_or_recover_compatibility_columns_once() {
        use std::sync::{Arc, Barrier};

        for missing in ["both", "context-only", "identity-only"] {
            let path = tempfile_path();
            {
                let db = Db::open(&path).unwrap();
                let conn = db.conn.lock().unwrap();
                if missing != "identity-only" {
                    conn.execute(
                        "ALTER TABLE sessions DROP COLUMN requested_context_window",
                        [],
                    )
                    .unwrap();
                }
                if missing != "context-only" {
                    conn.execute(
                        "ALTER TABLE sessions DROP COLUMN requested_model_identity",
                        [],
                    )
                    .unwrap();
                }
            }

            const OPENERS: usize = 12;
            let start = Arc::new(Barrier::new(OPENERS));
            let handles: Vec<_> = (0..OPENERS)
                .map(|_| {
                    let path = path.clone();
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        Db::open(path)
                    })
                })
                .collect();
            for handle in handles {
                handle
                    .join()
                    .expect("open thread panicked")
                    .expect("concurrent Db::open must not fail");
            }

            let db = Db::open(&path).unwrap();
            let conn = db.conn.lock().unwrap();
            let columns: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('sessions')
                      WHERE name IN ('requested_model_identity', 'requested_context_window')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let version: i32 = conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(columns, 2, "{missing}: each column exists exactly once");
            assert_eq!(version, 8, "{missing}: compatibility migration stays v8");
        }
    }

    #[test]
    fn a_switch_replaces_all_three_selection_columns_and_can_clear_the_window() {
        let db = Db::open(tempfile_path()).unwrap();
        let initial = selection("opus[1m]");
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "switch"),
            SpawnFacts {
                requested_selection: Some(&initial),
                ..Default::default()
            },
        )
        .unwrap();

        let switched = selection("sonnet");
        db.note_requested_model_selection("switch", &switched);
        assert_eq!(
            raw_selection_columns(&db, "switch"),
            (Some("sonnet".into()), Some("sonnet".into()), None),
        );
        dual_write_guard(&db, "switch").unwrap();

        // A following event carrying the same facts also uses replacement
        // semantics, closing the best-effort UPDATE failure window.
        db.record_event_with_spawn_facts(
            &ev("PreToolUse", "switch"),
            SpawnFacts {
                requested_selection: Some(&switched),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            db.load_recent_sessions(10).unwrap()[0].requested_selection,
            Some(switched.selection),
        );
    }

    #[test]
    fn dual_write_guard_detects_legacy_only_and_canonical_only_regressions() {
        let db = Db::open(tempfile_path()).unwrap();
        let persisted = selection("opus[1m]");
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "guard"),
            SpawnFacts {
                requested_selection: Some(&persisted),
                ..Default::default()
            },
        )
        .unwrap();
        dual_write_guard(&db, "guard").unwrap();

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET requested_model_identity = NULL,
                 requested_context_window = NULL WHERE id = 'guard'",
            [],
        )
        .unwrap();
        drop(conn);
        assert!(dual_write_guard(&db, "guard")
            .unwrap_err()
            .contains("canonical"));

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET requested_model = NULL,
                 requested_model_identity = 'opus', requested_context_window = 1000000
               WHERE id = 'guard'",
            [],
        )
        .unwrap();
        drop(conn);
        assert!(dual_write_guard(&db, "guard")
            .unwrap_err()
            .contains("legacy"));
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

    /// Account attribution is written at spawn time and survives a restart —
    /// and, critically, its THREE states stay three states.
    ///
    /// `Some("")` is the default account (a real answer), `Some(path)` is a
    /// named profile, and `None` is genuinely unknown: a session the daemon
    /// did not spawn. A UI that cannot tell the third from the first will bill
    /// unattributable sessions to the primary account, which is the specific
    /// lie this column exists to prevent — so it is asserted here, at the
    /// boundary where an over-eager `filter(|s| !s.is_empty())` would erase it.
    #[test]
    fn the_account_survives_a_restart_and_unknown_stays_unknown() {
        let db = Db::open(tempfile_path()).unwrap();
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "profile"),
            SpawnFacts {
                config_root: Some("/home/u/.claude/accounts/work"),
                ..Default::default()
            },
        )
        .unwrap();
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "default"),
            SpawnFacts {
                config_root: Some(""),
                ..Default::default()
            },
        )
        .unwrap();
        // Not spawned by the daemon: no attribution exists to record.
        db.record_event(&ev("SessionStart", "adopted-elsewhere"))
            .unwrap();

        let by_id: std::collections::HashMap<String, Option<String>> = db
            .load_recent_sessions(10)
            .unwrap()
            .into_iter()
            .map(|s| (s.id, s.config_root))
            .collect();
        assert_eq!(
            by_id["profile"].as_deref(),
            Some("/home/u/.claude/accounts/work"),
        );
        assert_eq!(
            by_id["default"].as_deref(),
            Some(""),
            "the default account is a KNOWN answer and must not read as unknown",
        );
        assert_eq!(
            by_id["adopted-elsewhere"], None,
            "a session the daemon did not spawn is unknown, not default —              NULL is the only honest value and nothing may backfill it",
        );
    }

    /// The spawn knows the account; the hooks that follow do not. A plain
    /// assignment on conflict would therefore blank it on the second event.
    #[test]
    fn the_account_is_stamped_once_and_never_blanked() {
        let db = Db::open(tempfile_path()).unwrap();
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "s1"),
            SpawnFacts {
                config_root: Some("/home/u/.claude/accounts/work"),
                ..Default::default()
            },
        )
        .unwrap();
        // Every later event carries no attribution at all.
        db.record_event(&ev("PreToolUse", "s1")).unwrap();
        db.record_event(&ev("Stop", "s1")).unwrap();
        assert_eq!(
            db.load_recent_sessions(10).unwrap()[0]
                .config_root
                .as_deref(),
            Some("/home/u/.claude/accounts/work"),
        );
    }

    /// A later hook naming a DIFFERENT transcript wins; a later hook naming
    /// A later hook naming a DIFFERENT transcript wins; a later hook naming
    /// none must not blank the column (most hooks carry it, but the upsert has
    /// to survive one that does not).
    #[test]
    fn transcript_path_takes_the_newest_and_is_never_blanked() {
        let db = Db::open(tempfile_path()).unwrap();
        let mut first = ev("SessionStart", "s1");
        first.payload.insert(
            "transcript_path".into(),
            json!("/home/u/.claude/projects/p/a.jsonl"),
        );
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
    /// `record_event_with_spawn_facts` stamping the value onto the row it
    /// creates that saves it.
    #[test]
    fn requested_model_survives_a_daemon_restart() {
        let tmp = tempfile_path();
        let db = Db::open(&tmp).unwrap();
        let selection =
            crate::session::windows::normalize_persisted_model_selection("opus[1m]").unwrap();

        // 1. Spawn records it. No row exists yet — this UPDATE hits nothing,
        //    and that is exactly the hole the second write closes.
        db.note_requested_model_selection("s1", &selection);
        let restored = db.load_recent_sessions(10).unwrap();
        assert!(
            restored.is_empty(),
            "no row yet: the spawn-time UPDATE has nothing to update"
        );

        // 2. The first hook event creates the row, carrying the model with it.
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "s1"),
            SpawnFacts {
                requested_selection: Some(&selection),
                ..Default::default()
            },
        )
        .unwrap();

        // 3. Restart: this is what `hydrate` gets handed.
        let restored = db.load_recent_sessions(10).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].requested_model.as_deref(), Some("opus[1m]"));
        assert_eq!(restored[0].requested_selection, Some(selection.selection));
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
        let selection =
            crate::session::windows::normalize_persisted_model_selection("sonnet[1m]").unwrap();
        db.record_event_with_spawn_facts(
            &ev("SessionStart", "s1"),
            SpawnFacts {
                requested_selection: Some(&selection),
                ..Default::default()
            },
        )
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
