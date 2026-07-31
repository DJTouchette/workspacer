//! Idempotent schema migration for the daemon's session/event persistence.

use anyhow::{bail, Result};
use rusqlite::Connection;

const USER_VERSION: i32 = 4;

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
