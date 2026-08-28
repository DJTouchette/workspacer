//! Idempotent schema migration for the daemon's session/event persistence.

use anyhow::{bail, Result};
use rusqlite::Connection;

const USER_VERSION: i32 = 7;

pub fn migrate(conn: &Connection) -> Result<()> {
    let current: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;

    // Downgrade guard: an older binary must NOT silently operate on a DB written
    // by a newer daemon — the newer schema may carry columns or semantics this
    // build doesn't understand, and treating it as "current" would corrupt data.
    // Refuse loudly instead.
    if current > USER_VERSION {
        bail!(
            "database schema version {current} is newer than this binary supports \
             ({USER_VERSION}); refusing to open — upgrade claudemon"
        );
    }

    // Step-wise, forward-only migrations. Each step runs inside ONE transaction
    // that covers both its body and its `user_version` bump (see `step`), so a
    // process death mid-migration leaves the DB either wholly before or wholly
    // after that step — never applied-but-unstamped, which for a non-replayable
    // step (`ALTER TABLE … ADD COLUMN`) would make every later `Db::open`, and
    // therefore daemon boot, fail forever with no way out.
    //
    // Write every new step so it is *also* safe to replay (`IF NOT EXISTS`, or a
    // catalog check like `add_heartbeat_provider`): DBs wedged by the pre-
    // transaction versions of this code still have to heal on the next boot.
    if current < 1 {
        step(conn, 1, |c| c.execute_batch(SCHEMA_V1))?;
    }
    if current < 2 {
        // v2: index `last_event_at` — the column `load_recent_sessions` orders by
        // and `Db::prune_archived` both filters and orders on. `IF NOT EXISTS`
        // keeps it safe on a DB that happened to already have it. The next real
        // migration adds its own `if current < 3 { … }` block right here.
        step(conn, 2, |c| c.execute_batch(SCHEMA_V2))?;
    }
    if current < 3 {
        // v3: keep-warm heartbeats — deliberately their own table, NOT rows in
        // `sessions`, so a warm ping can never surface anywhere sessions do
        // (sidebar, recent list, fleet). See daemon::heartbeat.
        step(conn, 3, |c| c.execute_batch(SCHEMA_V3))?;
    }
    if current < 4 {
        // v4: heartbeats grow a provider — Codex windows warm too.
        step(conn, 4, add_heartbeat_provider)?;
    }
    if current < 5 {
        // v5: sessions remember the model they were ASKED for. The `model`
        // column beside it holds what the transcript reported, which has the
        // `[1m]` marker stripped — so it cannot answer "was this a 1M session",
        // and a daemon restart used to revert a 1M session to the table's guess
        // for its stripped id.
        step(conn, 5, add_session_requested_model)?;
    }
    if current < 6 {
        // v6: sessions remember their TRANSCRIPT PATH. Without it every
        // rehydrated row folded usage from `None` and reported $0.00 / 0 tokens
        // forever — see `add_session_transcript_path` for the full story and
        // for why the backfill is real data rather than a guess.
        step(conn, 6, add_session_transcript_path)?;
    }
    if current < 7 {
        // v7: drop `total_cost_usd`. Written as a literal 0 by every insert
        // since v1, updated by nothing and read by nothing — it summed to
        // exactly 0.0 across all 806 rows on the author's machine. Cost is
        // folded from the transcript (`session::usage`), which is the single
        // authority; a second, permanently-stale copy in SQLite is a trap for
        // whoever reads it next, so the column goes rather than acquiring a
        // writer that would then have to be kept in agreement.
        step(conn, 7, drop_session_total_cost_usd)?;
    }
    Ok(())
}

/// Run one migration step atomically: its DDL and the `user_version` bump that
/// records it commit together or not at all. SQLite's DDL *is* transactional and
/// `PRAGMA user_version` is journaled with everything else, so the two can share
/// a transaction — which is the whole point, since the version is what tells the
/// next boot whether the body already ran.
fn step(
    conn: &Connection,
    version: i32,
    body: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let applied = body(conn).and_then(|()| conn.pragma_update(None, "user_version", version));
    match applied {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(err) => {
            // Best-effort: if the rollback itself fails the connection is beyond
            // saving and the original error is the one worth reporting.
            let _ = conn.execute_batch("ROLLBACK");
            Err(anyhow::Error::new(err).context(format!("applying schema v{version}")))
        }
    }
}

/// v4's body. `ALTER TABLE … ADD COLUMN` is the first non-replayable step in
/// this file — a second run errors with "duplicate column name" — so it checks
/// the catalog first. That makes the step idempotent for a DB left wedged
/// (column added, `user_version` still 3) by a kill in the window the old
/// two-statement migration left open.
fn add_heartbeat_provider(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT 1 FROM pragma_table_info('heartbeats') WHERE name = ?1")?;
    let present = stmt.exists(["provider"])?;
    drop(stmt);
    if present {
        return Ok(());
    }
    conn.execute_batch(SCHEMA_V4)
}

/// v5's body, catalog-checked for the same reason as
/// [`add_heartbeat_provider`]: `ALTER TABLE … ADD COLUMN` errors with
/// "duplicate column name" on a replay, and a DB left wedged by a kill in the
/// window between the DDL and the `user_version` bump has to heal on next boot.
fn add_session_requested_model(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1")?;
    let present = stmt.exists(["requested_model"])?;
    drop(stmt);
    if present {
        return Ok(());
    }
    conn.execute_batch(SCHEMA_V5)
}

/// v6's body: `sessions.transcript_path`, plus a backfill of every existing row.
///
/// Why this column has to exist. `usage::usage_for_session` folds a session's
/// cost and tokens out of `state.transcript_path`, and `usage_for_path(None)`
/// returns `Usage::default()` — all zeros. `transcript_path` was only ever
/// assigned from a LIVE hook event, so `SessionStore::hydrate` could not restore
/// it (there was no column to restore it from) and every rehydrated row folded
/// from `None`. Measured on the live daemon: 102 sessions listed, 94 of them
/// with `transcript_path: null` and `usage {cost_usd: 0.0}`. Only the handful
/// that instance had seen live carried real numbers. That is the $0.00 the user
/// sees at boot.
///
/// The backfill is NOT a guess. Every hook event Claude Code emits carries
/// `transcript_path`, and `events.payload_json` has been storing them verbatim
/// all along — all 806 session rows on the author's machine have a recoverable
/// path in their own event log. This takes the newest one per session, which is
/// the right choice if a session's path ever moved (a profile respawn).
///
/// Catalog-checked and therefore replay-safe, for the reason spelled out on
/// [`add_heartbeat_provider`].
fn add_session_transcript_path(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1")?;
    let present = stmt.exists(["transcript_path"])?;
    drop(stmt);
    if !present {
        conn.execute_batch(SCHEMA_V6)?;
    }
    // Runs whether or not the ALTER just happened: a DB wedged mid-step (column
    // added, version unstamped) still needs the backfill, and re-running it is a
    // no-op because of the `IS NULL` guard.
    conn.execute_batch(BACKFILL_V6)
}

/// v7's body. `ALTER TABLE … DROP COLUMN` (SQLite ≥ 3.35, and rusqlite's bundled
/// build is well past that) refuses on a second run, so the catalog decides.
/// Safe to drop: no index, view or trigger references the column, and nothing
/// outside this file has ever mentioned it.
fn drop_session_total_cost_usd(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1")?;
    let present = stmt.exists(["total_cost_usd"])?;
    drop(stmt);
    if !present {
        return Ok(());
    }
    conn.execute_batch(SCHEMA_V7)
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  project TEXT NOT NULL,
  cwd TEXT NOT NULL,
  worktree_path TEXT NOT NULL,
  branch TEXT,
  base_branch TEXT,
  model TEXT,
  state TEXT NOT NULL,
  pid INTEGER,
  created_at INTEGER NOT NULL,
  last_event_at INTEGER NOT NULL,
  total_cost_usd REAL DEFAULT 0,
  tool_call_count INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  event_type TEXT NOT NULL,
  tool_name TEXT,
  payload_json TEXT NOT NULL,
  FOREIGN KEY (session_id) REFERENCES sessions(id)
);
CREATE INDEX IF NOT EXISTS events_session_time ON events(session_id, timestamp DESC);
"#;

/// v2 migration. Additive index only. A column-adding upgrade does NOT belong in
/// a bare SQL const like this one: it goes through a step body that checks the
/// catalog first, the way `add_heartbeat_provider` does, so replaying it on a DB
/// where the column already exists is a no-op rather than an error.
const SCHEMA_V2: &str = r#"
CREATE INDEX IF NOT EXISTS sessions_last_event ON sessions(last_event_at DESC);
"#;

/// v3 migration: the keep-warm heartbeat log (one row per warm ping).
const SCHEMA_V3: &str = r#"
CREATE TABLE IF NOT EXISTS heartbeats (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at INTEGER NOT NULL,
  ok INTEGER NOT NULL,
  model TEXT NOT NULL,
  resets_at INTEGER,
  duration_ms INTEGER,
  error TEXT
);
CREATE INDEX IF NOT EXISTS heartbeats_at ON heartbeats(at DESC);
"#;

/// v4 migration: per-provider heartbeats (claude | codex).
const SCHEMA_V4: &str = r#"
ALTER TABLE heartbeats ADD COLUMN provider TEXT NOT NULL DEFAULT 'claude';
"#;

/// v5 migration: the model a session was ASKED for, which is the only carrier
/// of a `[1m]` window choice — Claude Code strips the marker from the id it
/// writes into the transcript. Nullable: old rows, and every session the daemon
/// did not spawn, genuinely do not know, and `NULL` says so.
const SCHEMA_V5: &str = r#"
ALTER TABLE sessions ADD COLUMN requested_model TEXT;
"#;

/// v6 migration: the transcript a session's usage is folded from. Nullable —
/// a session whose event log never carried one genuinely has no path, and
/// `NULL` says "unknown", which the fold already treats as such.
const SCHEMA_V6: &str = r#"
ALTER TABLE sessions ADD COLUMN transcript_path TEXT;
"#;

/// v6's backfill: recover each session's newest recorded transcript path from
/// its own persisted hook events. Only fills rows that have none, so it is
/// idempotent and never overwrites a live write.
const BACKFILL_V6: &str = r#"
UPDATE sessions SET transcript_path = (
  SELECT json_extract(e.payload_json, '$.transcript_path')
    FROM events e
   WHERE e.session_id = sessions.id
     AND json_extract(e.payload_json, '$.transcript_path') IS NOT NULL
   ORDER BY e.timestamp DESC, e.id DESC
   LIMIT 1
) WHERE transcript_path IS NULL;
"#;

/// v7 migration: retire the never-written, never-read `total_cost_usd`.
const SCHEMA_V7: &str = r#"
ALTER TABLE sessions DROP COLUMN total_cost_usd;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let v: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, USER_VERSION);
    }

    #[test]
    fn replays_a_step_whose_version_bump_was_lost() {
        // The wedge the pre-transaction migration could leave behind: v4's
        // `ALTER TABLE … ADD COLUMN` committed, the `user_version` bump didn't.
        // Re-running must heal the DB, not fail every boot with "duplicate
        // column name" — which used to make `Db::open` (and the daemon) dead.
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();

        migrate(&conn).unwrap();

        let v: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, USER_VERSION, "re-stamped to current");
        let cols: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('heartbeats') WHERE name = 'provider'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cols, 1, "provider column present exactly once");
    }

    #[test]
    fn a_failed_step_leaves_neither_body_nor_version() {
        // Atomicity: the body and the `user_version` bump share one transaction,
        // so a step that blows up rolls back to the version it started at and the
        // next run re-applies it from a clean state.
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.pragma_update(None, "user_version", 0).unwrap();

        let err = step(&conn, 1, |c| {
            c.execute_batch("CREATE TABLE half_applied (x INTEGER); SELECT bad_syntax(")
        })
        .unwrap_err();
        assert!(err.to_string().contains("applying schema v1"), "got: {err}");

        let v: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, 0, "version not bumped for a step that failed");
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'half_applied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0, "the step's partial work was rolled back");
    }

    #[test]
    fn core_tables_created() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        for table in ["sessions", "events"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table {} missing", table);
        }
    }

    /// v5 replays cleanly on a DB wedged with the column added but the version
    /// unstamped — the same hazard `add_heartbeat_provider` exists for, and the
    /// reason `ALTER TABLE … ADD COLUMN` never goes in a bare SQL const here.
    #[test]
    fn requested_model_migration_replays_on_a_wedged_db() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        // Simulate the kill: the DDL committed, the version bump did not.
        conn.pragma_update(None, "user_version", 4).unwrap();
        migrate(&conn).unwrap();
        let v: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, USER_VERSION, "healed instead of failing boot forever");
        let has: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1")
            .unwrap()
            .exists(["requested_model"])
            .unwrap();
        assert!(has, "sessions.requested_model exists after v5");
    }

    #[test]
    fn migrate_forward_from_older_version_preserves_data() {
        let conn = Connection::open_in_memory().unwrap();
        // Stamp the DB at an OLDER schema: apply only v1 and set user_version = 1.
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        // Seed a row that must survive the upgrade.
        conn.execute(
            "INSERT INTO sessions
               (id, name, project, cwd, worktree_path, state, created_at, last_event_at)
             VALUES ('s', 'n', 'p', '/w', '/w', 'working', 100, 100)",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let v: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, USER_VERSION, "migrated all the way forward");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions WHERE id = 's'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, 1, "existing data survived the upgrade");
        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'index' AND name = 'sessions_last_event'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1, "v2 index was created");
    }

    #[test]
    fn migrate_refuses_a_newer_database() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        // Simulate a future daemon having bumped the schema past this binary.
        conn.pragma_update(None, "user_version", USER_VERSION + 1)
            .unwrap();
        let err = migrate(&conn).unwrap_err();
        assert!(
            err.to_string().contains("newer than this binary"),
            "expected a downgrade-guard error, got: {err}"
        );
    }
}
