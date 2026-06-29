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
