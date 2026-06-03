/**
 * Historical metadata for old Claude sessions, persisted to SQLite for
 * analytics. One row per session (upserted by session_id) so totals survive
 * restarts and accumulate over the lifetime of the app. Capture is driven by
 * claudeSessionStore at turn boundaries (Stop) and finalised on SessionEnd.
 */
import { database } from './db';

export interface SessionHistoryRecord {
  sessionId: string;
  cwd: string;
  agentName: string;
  model: string | null;
  gitBranch: string;
  startedAt: string;   // ISO
  endedAt: string;     // ISO ('' while active)
  durationMs: number;
  inputTokens: number;
  outputTokens: number;
  costUSD: number;
  peakContext: number;
  toolCalls: number;
  messageCount: number;
  subagentCount: number;
  workflowRuns: number;
  workflowFailed: number;
  status: 'active' | 'ended';
}

export interface AnalyticsTotals {
  sessions: number;
  costUSD: number;
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  durationMs: number;
  workflowRuns: number;
}

export interface AnalyticsBucket {
  key: string;       // day (YYYY-MM-DD), cwd, or model
  sessions: number;
  costUSD: number;
  tokens: number;    // input + output
}

export interface AnalyticsSummary {
  totals: AnalyticsTotals;
  byDay: AnalyticsBucket[];     // chronological, recent window
  byProject: AnalyticsBucket[]; // top spenders
  byModel: AnalyticsBucket[];
}

class SessionHistoryStore {
  /** Insert or update a session's metadata row, keyed by session_id. */
  record(rec: SessionHistoryRecord): void {
    try {
      database.db
        .prepare(
          `INSERT INTO session_history (
             session_id, cwd, agent_name, model, git_branch, started_at, ended_at,
             duration_ms, input_tokens, output_tokens, cost_usd, peak_context,
             tool_calls, message_count, subagent_count, workflow_runs, workflow_failed,
             status, updated_at
           ) VALUES (
             @sessionId, @cwd, @agentName, @model, @gitBranch, @startedAt, @endedAt,
             @durationMs, @inputTokens, @outputTokens, @costUSD, @peakContext,
             @toolCalls, @messageCount, @subagentCount, @workflowRuns, @workflowFailed,
             @status, @updatedAt
           )
           ON CONFLICT(session_id) DO UPDATE SET
             cwd=excluded.cwd, agent_name=excluded.agent_name, model=excluded.model,
             git_branch=excluded.git_branch, ended_at=excluded.ended_at,
             duration_ms=excluded.duration_ms, input_tokens=excluded.input_tokens,
             output_tokens=excluded.output_tokens, cost_usd=excluded.cost_usd,
             peak_context=excluded.peak_context, tool_calls=excluded.tool_calls,
             message_count=excluded.message_count, subagent_count=excluded.subagent_count,
             workflow_runs=excluded.workflow_runs, workflow_failed=excluded.workflow_failed,
             status=excluded.status, updated_at=excluded.updated_at`,
        )
        .run({
          ...rec,
          model: rec.model ?? '',
          updatedAt: new Date().toISOString(),
        });
    } catch (err) {
      console.error('[SessionHistory] record failed:', err);
    }
  }

  /** Aggregate analytics across all recorded sessions. */
  summary(): AnalyticsSummary {
    const empty: AnalyticsSummary = {
      totals: { sessions: 0, costUSD: 0, inputTokens: 0, outputTokens: 0, toolCalls: 0, durationMs: 0, workflowRuns: 0 },
      byDay: [], byProject: [], byModel: [],
    };
    try {
      const db = database.db;
      const totals = db.prepare(
        `SELECT COUNT(*) AS sessions, COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens),0) AS inputTokens, COALESCE(SUM(output_tokens),0) AS outputTokens,
                COALESCE(SUM(tool_calls),0) AS toolCalls, COALESCE(SUM(duration_ms),0) AS durationMs,
                COALESCE(SUM(workflow_runs),0) AS workflowRuns
         FROM session_history`,
      ).get() as AnalyticsTotals;

      const byDay = db.prepare(
        `SELECT substr(started_at,1,10) AS key, COUNT(*) AS sessions,
                COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens+output_tokens),0) AS tokens
         FROM session_history
         WHERE started_at != ''
         GROUP BY key ORDER BY key DESC LIMIT 30`,
      ).all() as AnalyticsBucket[];

      const byProject = db.prepare(
        `SELECT cwd AS key, COUNT(*) AS sessions,
                COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens+output_tokens),0) AS tokens
         FROM session_history
         GROUP BY cwd ORDER BY costUSD DESC LIMIT 12`,
      ).all() as AnalyticsBucket[];

      const byModel = db.prepare(
        `SELECT CASE WHEN model='' THEN '(unknown)' ELSE model END AS key, COUNT(*) AS sessions,
                COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens+output_tokens),0) AS tokens
         FROM session_history
         GROUP BY model ORDER BY costUSD DESC LIMIT 12`,
      ).all() as AnalyticsBucket[];

      return { totals, byDay: byDay.reverse(), byProject, byModel };
    } catch (err) {
      console.error('[SessionHistory] summary failed:', err);
      return empty;
    }
  }

  /** Recent sessions, newest first, for the analytics table. */
  recent(limit = 100): SessionHistoryRecord[] {
    try {
      const rows = database.db.prepare(
        `SELECT session_id AS sessionId, cwd, agent_name AS agentName, model, git_branch AS gitBranch,
                started_at AS startedAt, ended_at AS endedAt, duration_ms AS durationMs,
                input_tokens AS inputTokens, output_tokens AS outputTokens, cost_usd AS costUSD,
                peak_context AS peakContext, tool_calls AS toolCalls, message_count AS messageCount,
                subagent_count AS subagentCount, workflow_runs AS workflowRuns,
                workflow_failed AS workflowFailed, status
         FROM session_history
         ORDER BY (CASE WHEN started_at='' THEN updated_at ELSE started_at END) DESC
         LIMIT ?`,
      ).all(limit) as SessionHistoryRecord[];
      return rows;
    } catch (err) {
      console.error('[SessionHistory] recent failed:', err);
      return [];
    }
  }
}

export const sessionHistory = new SessionHistoryStore();
