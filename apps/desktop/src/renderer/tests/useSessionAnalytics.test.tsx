/**
 * The analytics store has THREE answers, and collapsing any two of them is the
 * bug this hook exists to prevent:
 *
 *   - a figure it measured                → render it
 *   - a row it never recorded a figure for → render an absence
 *   - a store it cannot reach at all       → say so, render no figures
 *
 * The third is not hypothetical: a headless hub answers `analytics.summary`
 * with a well-formed ALL-ZERO payload carrying `unavailable: "headless"`
 * (services/hub/cmd/brain/handlers.go). Taken at face value that reads as
 * "$0.00 across 0 sessions", which is a measurement nobody took.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import {
  useSessionAnalytics,
  indexHistoryRows,
  RECENT_LIMIT,
} from '../src/hooks/useSessionAnalytics';
import type { SessionHistoryRecord } from '../src/types/analytics';

const rec = (over: Partial<SessionHistoryRecord> = {}): SessionHistoryRecord => ({
  sessionId: 's1',
  cwd: '/x',
  agentName: '',
  provider: 'claude',
  model: 'claude-opus-4-8',
  gitBranch: '',
  startedAt: '2026-08-01T00:00:00Z',
  endedAt: '',
  durationMs: 0,
  inputTokens: 900,
  outputTokens: 100,
  costUSD: 2.5,
  peakContext: 0,
  toolCalls: 4,
  messageCount: 0,
  subagentCount: 0,
  workflowRuns: 0,
  workflowFailed: 0,
  status: 'ended',
  ...over,
});

const summary = (costUSD: number, sessions: number, extra: Record<string, unknown> = {}) => ({
  totals: {
    sessions,
    costUSD,
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
  ...extra,
});

describe('indexHistoryRows — a stored zero is an absence, not a measurement', () => {
  it('keeps the figures a row actually recorded', () => {
    const { bySessionId, unrecordedSessions } = indexHistoryRows([rec()]);
    expect(bySessionId.s1).toMatchObject({ costUSD: 2.5, billedTokens: 1000, toolCalls: 4 });
    expect(unrecordedSessions).toBe(0);
  });

  it('reports an all-zero row as un-costed and omits its figures', () => {
    // session_history.cost_usd is `REAL DEFAULT 0` and never NULL, so a stored
    // 0 cannot be told apart from never-written — and a third of the rows on a
    // real machine are exactly that.
    const { bySessionId, unrecordedSessions } = indexHistoryRows([
      rec({ costUSD: 0, inputTokens: 0, outputTokens: 0 }),
    ]);
    expect(bySessionId.s1.costUSD).toBeUndefined();
    expect(bySessionId.s1.billedTokens).toBeUndefined();
    // The ROW is still known — only its usage is absent.
    expect(bySessionId.s1.model).toBe('claude-opus-4-8');
    expect(unrecordedSessions).toBe(1);
  });

  it('keeps tokens when only the cost is missing (a provider that bills no dollars)', () => {
    const { bySessionId, unrecordedSessions } = indexHistoryRows([rec({ costUSD: 0 })]);
    expect(bySessionId.s1.costUSD).toBeUndefined();
    expect(bySessionId.s1.billedTokens).toBe(1000);
    // Half-known is still recorded — it must not inflate the un-costed count.
    expect(unrecordedSessions).toBe(0);
  });

  it('counts un-costed rows separately so a total can say what it covers', () => {
    const { unrecordedSessions } = indexHistoryRows([
      rec({ sessionId: 'a' }),
      rec({ sessionId: 'b', costUSD: 0, inputTokens: 0, outputTokens: 0 }),
      rec({ sessionId: 'c', costUSD: 0, inputTokens: 0, outputTokens: 0 }),
    ]);
    expect(unrecordedSessions).toBe(2);
  });
});

describe('useSessionAnalytics — unreachable is not empty', () => {
  beforeEach(() => {
    (window.electronAPI as any).analyticsSummary = vi.fn();
    (window.electronAPI as any).analyticsRecent = vi.fn();
  });

  it('surfaces the real lifetime totals when the desktop store answers', async () => {
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(summary(14892.67, 747));
    (window.electronAPI.analyticsRecent as any).mockResolvedValue([rec()]);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.summary?.totals.costUSD).toBe(14892.67);
    expect(result.current.summary?.totals.sessions).toBe(747);
    expect(result.current.bySessionId.s1.costUSD).toBe(2.5);
    expect(result.current.unavailable).toBeNull();
  });

  it("refuses a headless stub's all-zero payload — null summary, unavailable set", async () => {
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(
      summary(0, 0, { unavailable: 'headless' }),
    );
    (window.electronAPI.analyticsRecent as any).mockResolvedValue([]);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    // NOT `{ costUSD: 0, sessions: 0 }` — that is a measurement nobody took.
    expect(result.current.summary).toBeNull();
    expect(result.current.unavailable).toBe('headless');
  });

  it('reports a rejected fetch as unavailable rather than as an empty history', async () => {
    (window.electronAPI.analyticsSummary as any).mockRejectedValue(new Error('no provider'));
    (window.electronAPI.analyticsRecent as any).mockResolvedValue([]);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.summary).toBeNull();
    expect(result.current.unavailable).toMatch(/no provider/);
  });

  it("treats main's own read failure marker exactly like the headless stub's", async () => {
    // sessionHistory.summary() sets `unavailable` on the all-zero fallback it
    // returns when the SQLite read throws — same field, same contract.
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(
      summary(0, 0, { unavailable: 'database unavailable' }),
    );
    (window.electronAPI.analyticsRecent as any).mockResolvedValue([]);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.summary).toBeNull();
    expect(result.current.unavailable).toBe('database unavailable');
  });

  it('reports a rejected ROW read as unavailable — sessionHistory.recent now throws rather than answering []', async () => {
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(summary(14892.67, 747));
    (window.electronAPI.analyticsRecent as any).mockRejectedValue(
      new Error('database unavailable'),
    );
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    // The totals were readable, but the rows were not, and per-row absences
    // built from a failed read would each read as "nothing was recorded".
    expect(result.current.unavailable).toMatch(/database unavailable/);
    expect(result.current.bySessionId).toEqual({});
    expect(result.current.summary).toBeNull();
  });

  it('treats a payload with no totals as unavailable rather than reading zeros off it', async () => {
    // A peer or a stubbed transport can answer `{}` — well-formed JSON with no
    // measurement in it. Reading `.totals.costUSD` off that throws, and
    // guarding it with `?? 0` would invent a $0.00 nobody reported.
    (window.electronAPI.analyticsSummary as any).mockResolvedValue({});
    (window.electronAPI.analyticsRecent as any).mockResolvedValue([]);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.summary).toBeNull();
    expect(result.current.unavailable).toBe('no totals in response');
  });

  it('flags the un-costed count as a floor when the row read hits its cap', async () => {
    // The un-costed count is derived from the rows READ; the session count
    // comes from the whole store. Once the read is capped those denominators
    // differ, and a surface printing them side by side has to say so.
    const rows = Array.from({ length: RECENT_LIMIT }, (_, i) =>
      rec({ sessionId: `s${i}`, costUSD: 0, inputTokens: 0, outputTokens: 0 }),
    );
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(summary(14892.67, 5000));
    (window.electronAPI.analyticsRecent as any).mockResolvedValue(rows);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.unrecordedSessions).toBe(RECENT_LIMIT);
    expect(result.current.unrecordedComplete).toBe(false);
  });

  it('prefers the store’s own un-costed count over the capped row-derived floor', async () => {
    // Same capped read as above, but the store reported the exact figure. It
    // is counted over EVERY row, so it shares a denominator with the session
    // count beside it and the "at least" hedge is no longer warranted.
    const rows = Array.from({ length: RECENT_LIMIT }, (_, i) =>
      rec({ sessionId: `s${i}`, costUSD: 0, inputTokens: 0, outputTokens: 0 }),
    );
    const sum = summary(14892.67, 5000) as any;
    sum.totals.unrecordedSessions = 231;
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(sum);
    (window.electronAPI.analyticsRecent as any).mockResolvedValue(rows);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.unrecordedSessions).toBe(231);
    expect(result.current.unrecordedComplete).toBe(true);
  });

  it('does not read a missing un-costed count as zero un-costed rows', async () => {
    // A source that omits the field (the headless stub, an older main) says
    // nothing about un-costed rows. Falling through to the rows actually read
    // is the honest answer; treating undefined as 0 would claim every one of
    // the 5000 sessions was costed.
    const rows = Array.from({ length: RECENT_LIMIT }, (_, i) =>
      rec({ sessionId: `s${i}`, costUSD: 0, inputTokens: 0, outputTokens: 0 }),
    );
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(summary(14892.67, 5000));
    (window.electronAPI.analyticsRecent as any).mockResolvedValue(rows);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.unrecordedSessions).toBe(RECENT_LIMIT);
    expect(result.current.unrecordedComplete).toBe(false);
  });

  it('a genuine zero total from a reachable store is kept as a measurement', async () => {
    // A brand-new install: the store answers, and it truly has nothing. That
    // IS $0.00 across 0 sessions, and must not be reported as unavailable.
    (window.electronAPI.analyticsSummary as any).mockResolvedValue(summary(0, 0));
    (window.electronAPI.analyticsRecent as any).mockResolvedValue([]);
    const { result } = renderHook(() => useSessionAnalytics());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.summary?.totals.costUSD).toBe(0);
    expect(result.current.unavailable).toBeNull();
    // An uncapped read: the un-costed count is a count, not a floor.
    expect(result.current.unrecordedComplete).toBe(true);
  });
});
