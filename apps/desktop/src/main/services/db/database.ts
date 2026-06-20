import * as path from 'path';
import * as fs from 'fs';
import type BetterSqlite3 from 'better-sqlite3';
import { getConfigDir } from '../configService';

// Native module — use require to avoid ESM/bundler issues
const Database = require('better-sqlite3') as typeof BetterSqlite3;

const MIGRATION_V1 = `
CREATE TABLE IF NOT EXISTS issues (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  key TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT DEFAULT '',
  status TEXT DEFAULT '',
  status_category TEXT DEFAULT 'todo',
  assignee TEXT,
  priority TEXT,
  type TEXT DEFAULT '',
  labels TEXT DEFAULT '[]',
  project_key TEXT DEFAULT '',
  url TEXT DEFAULT '',
  parent_key TEXT,
  created TEXT DEFAULT '',
  updated TEXT DEFAULT '',
  synced_at TEXT NOT NULL,
  UNIQUE(account_id, key)
);

CREATE TABLE IF NOT EXISTS branches (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_path TEXT NOT NULL,
  name TEXT NOT NULL,
  is_remote INTEGER DEFAULT 0,
  head_sha TEXT,
  updated_at TEXT NOT NULL,
  UNIQUE(repo_path, name, is_remote)
);

CREATE TABLE IF NOT EXISTS pull_requests (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  number INTEGER,
  title TEXT NOT NULL,
  description TEXT DEFAULT '',
  status TEXT DEFAULT 'open',
  source_branch TEXT DEFAULT '',
  target_branch TEXT DEFAULT '',
  author TEXT,
  url TEXT DEFAULT '',
  created TEXT DEFAULT '',
  updated TEXT DEFAULT '',
  synced_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pipelines (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  name TEXT NOT NULL,
  status TEXT DEFAULT '',
  source_branch TEXT DEFAULT '',
  commit_sha TEXT,
  url TEXT DEFAULT '',
  started_at TEXT,
  finished_at TEXT,
  synced_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS issue_links (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  issue_key TEXT NOT NULL,
  link_type TEXT NOT NULL,
  link_id TEXT NOT NULL,
  link_label TEXT DEFAULT '',
  created_at TEXT NOT NULL,
  UNIQUE(issue_key, link_type, link_id)
);

CREATE INDEX IF NOT EXISTS idx_issues_key ON issues(key);
CREATE INDEX IF NOT EXISTS idx_issues_account ON issues(account_id);
CREATE INDEX IF NOT EXISTS idx_issues_parent ON issues(parent_key);
CREATE INDEX IF NOT EXISTS idx_issues_project ON issues(project_key, account_id);
CREATE INDEX IF NOT EXISTS idx_branches_repo ON branches(repo_path);
CREATE INDEX IF NOT EXISTS idx_prs_account ON pull_requests(account_id);
CREATE INDEX IF NOT EXISTS idx_prs_source ON pull_requests(source_branch);
CREATE INDEX IF NOT EXISTS idx_pipelines_account ON pipelines(account_id);
CREATE INDEX IF NOT EXISTS idx_pipelines_branch ON pipelines(source_branch);
CREATE INDEX IF NOT EXISTS idx_issue_links_key ON issue_links(issue_key);
CREATE INDEX IF NOT EXISTS idx_issue_links_target ON issue_links(link_type, link_id);
`;

// v2: historical metadata for finished/old Claude sessions, for analytics.
// One row per session (upserted by session_id), append-only over the app's life.
const MIGRATION_V2 = `
CREATE TABLE IF NOT EXISTS session_history (
  session_id TEXT PRIMARY KEY,
  cwd TEXT DEFAULT '',
  agent_name TEXT DEFAULT '',
  model TEXT DEFAULT '',
  git_branch TEXT DEFAULT '',
  started_at TEXT DEFAULT '',
  ended_at TEXT DEFAULT '',
  duration_ms INTEGER DEFAULT 0,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cost_usd REAL DEFAULT 0,
  peak_context INTEGER DEFAULT 0,
  tool_calls INTEGER DEFAULT 0,
  message_count INTEGER DEFAULT 0,
  subagent_count INTEGER DEFAULT 0,
  workflow_runs INTEGER DEFAULT 0,
  workflow_failed INTEGER DEFAULT 0,
  status TEXT DEFAULT 'active',
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_session_history_started ON session_history(started_at);
CREATE INDEX IF NOT EXISTS idx_session_history_cwd ON session_history(cwd);
CREATE INDEX IF NOT EXISTS idx_session_history_model ON session_history(model);
`;

function getDbPath(): string {
  return path.join(getConfigDir(), 'workspacer.db');
}

export class DatabaseService {
  private _db: BetterSqlite3.Database | null = null;

  get db(): BetterSqlite3.Database {
    if (!this._db) {
      this.open();
    }
    return this._db!;
  }

  /** Open the database, creating the directory and running migrations if needed. */
  open(): void {
    if (this._db) return;

    const dbPath = getDbPath();
    const dir = path.dirname(dbPath);
    fs.mkdirSync(dir, { recursive: true });

    this._db = new Database(dbPath);

    // Performance pragmas
    this._db.pragma('journal_mode = WAL');
    this._db.pragma('foreign_keys = ON');
    this._db.pragma('busy_timeout = 5000');

    this.migrate();
    console.log('[DatabaseService] opened', dbPath);
  }

  /** Run schema migrations. */
  private migrate(): void {
    const db = this._db!;

    // Track schema version
    db.exec(`CREATE TABLE IF NOT EXISTS _migrations (
      version INTEGER PRIMARY KEY,
      applied_at TEXT NOT NULL
    )`);

    const current = db.prepare('SELECT MAX(version) as v FROM _migrations').get() as
      | { v: number | null }
      | undefined;
    const currentVersion = current?.v ?? 0;

    if (currentVersion < 1) {
      db.transaction(() => {
        db.exec(MIGRATION_V1);
        db.prepare('INSERT INTO _migrations (version, applied_at) VALUES (?, ?)').run(
          1,
          new Date().toISOString(),
        );
      })();
      console.log('[DatabaseService] applied migration v1');
    }

    if (currentVersion < 2) {
      db.transaction(() => {
        db.exec(MIGRATION_V2);
        db.prepare('INSERT INTO _migrations (version, applied_at) VALUES (?, ?)').run(
          2,
          new Date().toISOString(),
        );
      })();
      console.log('[DatabaseService] applied migration v2');
    }
  }

  /** Close the database connection. Call on app shutdown. */
  close(): void {
    if (this._db) {
      this._db.close();
      this._db = null;
      console.log('[DatabaseService] closed');
    }
  }
}

export const database = new DatabaseService();
