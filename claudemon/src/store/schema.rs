//! Idempotent schema migration. Tables match §10 of the v2 spec.
//!
//! Phase 0 populates only `sessions` and `events`. The remaining tables
//! (`items`, `pending_decisions`, `asks`, `events_fts`) are created now so
//! later phases don't have to touch the schema version.

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

CREATE TABLE IF NOT EXISTS items (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  state TEXT NOT NULL,
  priority INTEGER NOT NULL,
  kind TEXT NOT NULL,
  summary TEXT,
  context_paragraph TEXT,
  next_action TEXT,
  triggering_event_id INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  resolved_at INTEGER,
  snoozed_until INTEGER,
  snoozed_on_event TEXT,
  flagged INTEGER DEFAULT 0,
  FOREIGN KEY (session_id) REFERENCES sessions(id),
  FOREIGN KEY (triggering_event_id) REFERENCES events(id)
);
CREATE INDEX IF NOT EXISTS items_state_priority
  ON items(state, priority DESC, updated_at DESC);

CREATE TABLE IF NOT EXISTS pending_decisions (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  item_id TEXT,
  kind TEXT NOT NULL,
  tool_name TEXT,
  tool_input_json TEXT,
  prompt_text TEXT,
  created_at INTEGER NOT NULL,
  resolved_at INTEGER,
  resolution TEXT,
  FOREIGN KEY (session_id) REFERENCES sessions(id),
  FOREIGN KEY (item_id) REFERENCES items(id)
);

CREATE TABLE IF NOT EXISTS asks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  scope TEXT,
  scope_ref TEXT,
  question TEXT NOT NULL,
  response TEXT,
  tool_calls_json TEXT,
  cost_usd REAL,
  duration_ms INTEGER,
  created_at INTEGER NOT NULL,
  completed_at INTEGER
);

CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
  session_id UNINDEXED, timestamp UNINDEXED, content,
  content='', tokenize='porter unicode61'
);
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
    fn all_tables_created() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        for table in [
            "sessions",
            "events",
            "items",
            "pending_decisions",
            "asks",
            "events_fts",
        ] {
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
