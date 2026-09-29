CREATE TABLE IF NOT EXISTS session_history (
 session_id TEXT PRIMARY KEY,cwd TEXT DEFAULT '',agent_name TEXT DEFAULT '',model TEXT DEFAULT '',git_branch TEXT DEFAULT '',
 started_at TEXT DEFAULT '',ended_at TEXT DEFAULT '',duration_ms INTEGER DEFAULT 0,input_tokens INTEGER DEFAULT 0,output_tokens INTEGER DEFAULT 0,
 cost_usd REAL DEFAULT 0,peak_context INTEGER DEFAULT 0,tool_calls INTEGER DEFAULT 0,message_count INTEGER DEFAULT 0,subagent_count INTEGER DEFAULT 0,
 workflow_runs INTEGER DEFAULT 0,workflow_failed INTEGER DEFAULT 0,status TEXT DEFAULT 'active',updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_history_started ON session_history(started_at);
CREATE INDEX IF NOT EXISTS idx_session_history_cwd ON session_history(cwd);
CREATE INDEX IF NOT EXISTS idx_session_history_model ON session_history(model);
CREATE TABLE IF NOT EXISTS session_model_usage (
 session_id TEXT NOT NULL,model TEXT NOT NULL,input_tokens INTEGER DEFAULT 0,output_tokens INTEGER DEFAULT 0,cost_usd REAL DEFAULT 0,updated_at TEXT NOT NULL,
 PRIMARY KEY(session_id,model)
);
CREATE INDEX IF NOT EXISTS idx_session_model_usage_model ON session_model_usage(model);
CREATE TABLE IF NOT EXISTS analytics_sources (session_id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL);
