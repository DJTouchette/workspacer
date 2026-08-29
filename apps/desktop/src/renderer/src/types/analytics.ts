/** Mirrors src/main/services/sessionHistory.ts. */

export interface AnalyticsTotals {
  sessions: number;
  costUSD: number;
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  durationMs: number;
  workflowRuns: number;
  /**
   * How many of `sessions` carry no usage at all — `cost_usd`, `input_tokens`
   * and `output_tokens` all zero.
   *
   * The columns are `REAL/INTEGER DEFAULT 0` and never NULL, so a row that was
   * created and never had usage written to it is indistinguishable from one
   * measured at zero; ~31% of the rows on a real machine are that. Counting
   * them is the only honest way to say what the lifetime total covers — the
   * figure is a sum over `sessions - unrecordedSessions` rows, not over all of
   * them.
   *
   * OPTIONAL because a source that does not compute it must not be read as
   * "zero un-costed rows": the headless brain stub and any older desktop main
   * both answer without it, and undefined sends the consumer back to counting
   * the rows it actually read (a floor, labelled as one).
   */
  unrecordedSessions?: number;
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
