/** Mirrors src/main/services/sessionHistory.ts. */

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
  key: string;
  sessions: number;
  costUSD: number;
  tokens: number;
}

export interface AnalyticsSummary {
  totals: AnalyticsTotals;
  byDay: AnalyticsBucket[];
  byProject: AnalyticsBucket[];
  byModel: AnalyticsBucket[];
  /** Split by coding-agent backend (claude/codex/opencode); always all rows. */
  byProvider: AnalyticsBucket[];
  /** Set only when the store could NOT be read — by main when the SQLite read
   *  throws, and by the headless brain stub (`"headless"`). The zeros beside
   *  it are filler; see useSessionAnalytics, which turns this into an absence
   *  rather than letting it render as a measured $0.00. */
  unavailable?: string;
}

export interface SessionHistoryRecord {
  sessionId: string;
  cwd: string;
  agentName: string;
  /** Coding-agent backend ('claude' | 'codex' | 'opencode'). '' ⇒ claude. */
  provider: string;
  model: string | null;
  gitBranch: string;
  startedAt: string;
  endedAt: string;
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
