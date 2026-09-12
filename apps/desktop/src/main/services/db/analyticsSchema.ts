// v2: historical metadata for finished/old Claude sessions, for analytics.
// One row per session (upserted by session_id), append-only over the app's life.
export const MIGRATION_V2 = `
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

// Tag each analytics row with the coding-agent backend, so usage can be split
// by provider (Claude / Codex / OpenCode) or seen combined.
export const MIGRATION_V3 = `
ALTER TABLE session_history ADD COLUMN provider TEXT DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_session_history_provider ON session_history(provider);
`;

// v4: per-model usage split within a session (main thread + subagent turns),
// so the "By model" analytics attribute tokens/cost to the model that actually
// spent them instead of pinning a whole session to its last main-thread model.
export const MIGRATION_V4 = `
CREATE TABLE IF NOT EXISTS session_model_usage (
  session_id TEXT NOT NULL,
  model TEXT NOT NULL,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cost_usd REAL DEFAULT 0,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (session_id, model)
);

CREATE INDEX IF NOT EXISTS idx_session_model_usage_model ON session_model_usage(model);
`;
