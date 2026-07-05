//! Idempotent schema migration for the daemon's session/event persistence.

use anyhow::Result;
use rusqlite::Connection;

const USER_VERSION: i32 = 1;

pub fn migrate(conn: &Connection) -> Result<()> {
    let current: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current >= USER_VERSION {
        return Ok(());
    }
    conn.execute_batch(SCHEMA_V1)?;
    conn.pragma_update(None, "user_version", USER_VERSION)?;
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
}
