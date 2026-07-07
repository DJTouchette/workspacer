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
  /** Coding-agent backend ('claude' | 'codex' | 'opencode'). '' ⇒ claude (legacy). */
  provider: string;
  model: string | null;
  gitBranch: string;
  startedAt: string; // ISO
  endedAt: string; // ISO ('' while active)
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
  key: string; // day (YYYY-MM-DD), cwd, or model
  sessions: number;
  costUSD: number;
  tokens: number; // input + output
}

export interface AnalyticsSummary {
  totals: AnalyticsTotals;
  byDay: AnalyticsBucket[]; // chronological, recent window
  byProject: AnalyticsBucket[]; // top spenders
  byModel: AnalyticsBucket[];
  byProvider: AnalyticsBucket[]; // split by coding-agent backend (always all)
}

/** SQL fragment + bind params for the optional provider / time-range filters.
 *  Legacy rows have provider='' which we treat as 'claude'. `since` is an ISO
 *  timestamp; started_at is stored as ISO so plain string compare orders it.
 *  `alias` qualifies the filtered columns when the query joins tables. */
function rowFilter(
  provider?: string,
  since?: string,
  alias = '',
): { where: string; params: Record<string, string> } {
  const col = alias ? `${alias}.` : '';
  const clauses: string[] = [];
  const params: Record<string, string> = {};
  if (provider === 'claude') clauses.push(`(${col}provider='' OR ${col}provider='claude')`);
  else if (provider) {
    clauses.push(`${col}provider=@provider`);
    params.provider = provider;
  }
  if (since) {
    clauses.push(`${col}started_at >= @since`);
    params.since = since;
  }
  return { where: clauses.length > 0 ? `WHERE ${clauses.join(' AND ')}` : '', params };
}

class SessionHistoryStore {
  /** Insert or update a session's metadata row, keyed by session_id. */
  record(rec: SessionHistoryRecord): void {
    try {
      database.db
        .prepare(
          `INSERT INTO session_history (
             session_id, cwd, agent_name, provider, model, git_branch, started_at, ended_at,
             duration_ms, input_tokens, output_tokens, cost_usd, peak_context,
             tool_calls, message_count, subagent_count, workflow_runs, workflow_failed,
             status, updated_at
           ) VALUES (
             @sessionId, @cwd, @agentName, @provider, @model, @gitBranch, @startedAt, @endedAt,
             @durationMs, @inputTokens, @outputTokens, @costUSD, @peakContext,
             @toolCalls, @messageCount, @subagentCount, @workflowRuns, @workflowFailed,
             @status, @updatedAt
           )
           ON CONFLICT(session_id) DO UPDATE SET
             cwd=excluded.cwd, agent_name=excluded.agent_name, provider=excluded.provider, model=excluded.model,
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
          provider: rec.provider ?? '',
          model: rec.model ?? '',
          updatedAt: new Date().toISOString(),
        });
    } catch (err) {
      console.error('[SessionHistory] record failed:', err);
    }
  }

  /** Upsert a session's per-model usage split (main thread + subagent turns).
   *  Values are cumulative session totals per model, so a plain replace is
   *  idempotent under repeated snapshots. */
  recordModels(
    sessionId: string,
    models: Record<string, { inputTokens: number; outputTokens: number; costUSD: number }>,
  ): void {
    const entries = Object.entries(models);
    if (entries.length === 0) return;
    try {
      const stmt = database.db.prepare(
        `INSERT INTO session_model_usage (session_id, model, input_tokens, output_tokens, cost_usd, updated_at)
         VALUES (@sessionId, @model, @inputTokens, @outputTokens, @costUSD, @updatedAt)
         ON CONFLICT(session_id, model) DO UPDATE SET
           input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
           cost_usd=excluded.cost_usd, updated_at=excluded.updated_at`,
      );
      const updatedAt = new Date().toISOString();
      for (const [model, m] of entries) {
        stmt.run({
          sessionId,
          model,
          inputTokens: m.inputTokens,
          outputTokens: m.outputTokens,
          costUSD: m.costUSD,
          updatedAt,
        });
      }
    } catch (err) {
      console.error('[SessionHistory] recordModels failed:', err);
    }
  }

  /** Aggregate analytics. With `provider`, the totals/breakdowns are scoped to
   *  that backend; `byProvider` is always computed across all providers so the
   *  split is visible even while filtered. With `since` (ISO), everything —
   *  including `byProvider` — is scoped to sessions started at/after it. */
  summary(provider?: string, since?: string): AnalyticsSummary {
    const empty: AnalyticsSummary = {
      totals: {
        sessions: 0,
        costUSD: 0,
        inputTokens: 0,
        outputTokens: 0,
        toolCalls: 0,
        durationMs: 0,
        workflowRuns: 0,
      },
      byDay: [],
      byProject: [],
      byModel: [],
      byProvider: [],
    };
    try {
      const db = database.db;
      const { where, params } = rowFilter(provider, since);
      const and = where ? `${where} AND` : 'WHERE';
      // One daily bar per day in the window; unbounded ranges cap at 90 bars.
      const dayLimit = since
        ? Math.max(1, Math.ceil((Date.now() - Date.parse(since)) / 86_400_000))
        : 90;

      const totals = db
        .prepare(
          `SELECT COUNT(*) AS sessions, COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens),0) AS inputTokens, COALESCE(SUM(output_tokens),0) AS outputTokens,
                COALESCE(SUM(tool_calls),0) AS toolCalls, COALESCE(SUM(duration_ms),0) AS durationMs,
                COALESCE(SUM(workflow_runs),0) AS workflowRuns
         FROM session_history ${where}`,
        )
        .get(params) as AnalyticsTotals;

      const byDay = db
        .prepare(
          `SELECT substr(started_at,1,10) AS key, COUNT(*) AS sessions,
                COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens+output_tokens),0) AS tokens
         FROM session_history
         ${and} started_at != ''
         GROUP BY key ORDER BY key DESC LIMIT @dayLimit`,
        )
        .all({ ...params, dayLimit }) as AnalyticsBucket[];

      const byProject = db
        .prepare(
          `SELECT cwd AS key, COUNT(*) AS sessions,
                COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens+output_tokens),0) AS tokens
         FROM session_history ${where}
         GROUP BY cwd ORDER BY costUSD DESC LIMIT 12`,
        )
        .all(params) as AnalyticsBucket[];

      // Per-model: prefer the session_model_usage split (attributes subagent
      // turns to the model that ran them); sessions recorded before the split
      // existed fall back to their single-model session_history row.
      const aliased = rowFilter(provider, since, 'sh');
      const aliasedAnd = aliased.where ? `${aliased.where} AND` : 'WHERE';
      const byModel = db
        .prepare(
          `SELECT CASE WHEN model='' THEN '(unknown)' ELSE model END AS key,
                COUNT(DISTINCT session_id) AS sessions,
                COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(tokens),0) AS tokens
         FROM (
           SELECT smu.session_id, smu.model, smu.cost_usd,
                  smu.input_tokens + smu.output_tokens AS tokens
           FROM session_model_usage smu
           JOIN session_history sh ON sh.session_id = smu.session_id
           ${aliased.where}
           UNION ALL
           SELECT sh.session_id, sh.model, sh.cost_usd,
                  sh.input_tokens + sh.output_tokens AS tokens
           FROM session_history sh
           ${aliasedAnd} NOT EXISTS (
             SELECT 1 FROM session_model_usage smu WHERE smu.session_id = sh.session_id
           )
         )
         GROUP BY key ORDER BY costUSD DESC LIMIT 12`,
        )
        .all(aliased.params) as AnalyticsBucket[];

      // Provider split — all providers ('' counts as claude), same time window.
      const providerTime = rowFilter(undefined, since);
      const byProvider = db
        .prepare(
          `SELECT CASE WHEN provider='' THEN 'claude' ELSE provider END AS key, COUNT(*) AS sessions,
                COALESCE(SUM(cost_usd),0) AS costUSD,
                COALESCE(SUM(input_tokens+output_tokens),0) AS tokens
         FROM session_history ${providerTime.where}
         GROUP BY key ORDER BY costUSD DESC`,
        )
        .all(providerTime.params) as AnalyticsBucket[];

      return { totals, byDay: byDay.reverse(), byProject, byModel, byProvider };
    } catch (err) {
      console.error('[SessionHistory] summary failed:', err);
      return empty;
    }
  }

  /** Recent sessions, newest first, for the analytics table. Optionally scoped
   *  to one provider and/or a time range (ISO `since`). */
  recent(limit = 100, provider?: string, since?: string): SessionHistoryRecord[] {
    try {
      const { where, params } = rowFilter(provider, since);
      const rows = database.db
        .prepare(
          `SELECT session_id AS sessionId, cwd, agent_name AS agentName, provider, model, git_branch AS gitBranch,
                started_at AS startedAt, ended_at AS endedAt, duration_ms AS durationMs,
                input_tokens AS inputTokens, output_tokens AS outputTokens, cost_usd AS costUSD,
                peak_context AS peakContext, tool_calls AS toolCalls, message_count AS messageCount,
                subagent_count AS subagentCount, workflow_runs AS workflowRuns,
                workflow_failed AS workflowFailed, status
         FROM session_history ${where}
         ORDER BY (CASE WHEN started_at='' THEN updated_at ELSE started_at END) DESC
         LIMIT @limit`,
        )
        .all({ ...params, limit }) as SessionHistoryRecord[];
      return rows;
    } catch (err) {
      console.error('[SessionHistory] recent failed:', err);
      return [];
    }
  }
}

export const sessionHistory = new SessionHistoryStore();
