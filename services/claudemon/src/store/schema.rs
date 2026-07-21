//! Idempotent schema migration for the daemon's session/event persistence.

use anyhow::{bail, Result};
use rusqlite::Connection;

const USER_VERSION: i32 = 3;

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

    // Step-wise, forward-only migrations. Each block advances the on-disk
    // `user_version` by exactly one, so a partial run resumes cleanly and every
    // future upgrade is exercised by construction rather than being an untested
    // monolith on the day it first ships.
    if current < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    if current < 2 {
        // v2: index `last_event_at` — the column `load_recent_sessions` orders by
        // and `Db::prune_archived` both filters and orders on. `IF NOT EXISTS`
        // keeps it safe on a DB that happened to already have it. The next real
        // migration adds its own `if current < 3 { … }` block right here.
        conn.execute_batch(SCHEMA_V2)?;
        conn.pragma_update(None, "user_version", 2)?;
    }
    if current < 3 {
        // v3: keep-warm heartbeats — deliberately their own table, NOT rows in
        // `sessions`, so a warm ping can never surface anywhere sessions do
        // (sidebar, recent list, fleet). See daemon::heartbeat.
        conn.execute_batch(SCHEMA_V3)?;
        conn.pragma_update(None, "user_version", 3)?;
    }
    Ok(())
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

/// v2 migration. Additive index only — a template for the real column-adding
/// upgrades to come (`ALTER TABLE … ADD COLUMN …` goes in a block like this).
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
