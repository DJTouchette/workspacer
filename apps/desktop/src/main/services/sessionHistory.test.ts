/**
 * The analytics store must not answer "I could not read this" with the same
 * shape it answers "there is nothing here". Both are legitimate answers and
 * they mean opposite things: one is an outage, the other is an idle install.
 *
 * The failure path used to `return empty` — an all-zero AnalyticsSummary with
 * no marker — which every downstream surface rendered as a confident
 * "$0.00 across 0 sessions" on a machine whose database holds five figures.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const state = vi.hoisted(() => ({ db: null as unknown }));

vi.mock('./db', () => ({
  database: {
    get db() {
      if (!state.db) throw new Error('database unavailable');
      return state.db;
    },
  },
}));

const { sessionHistory } = await import('./sessionHistory');

/** A store that answers every query truthfully with nothing — a fresh install. */
function emptyStore() {
  return {
    prepare: () => ({
      get: () => ({
        sessions: 0,
        costUSD: 0,
        inputTokens: 0,
        outputTokens: 0,
        toolCalls: 0,
        durationMs: 0,
        workflowRuns: 0,
      }),
      all: () => [],
    }),
  };
}

describe('sessionHistory.summary — unreadable is not empty', () => {
  beforeEach(() => {
    state.db = null;
  });

  it('marks the all-zero fallback as unavailable when the read throws', () => {
    const out = sessionHistory.summary();
    expect(out.totals.costUSD).toBe(0);
    // The zeros are filler. Without this marker they are indistinguishable
    // from a real reading of zero.
    expect(out.unavailable).toMatch(/database unavailable/);
  });

  it('leaves a genuine zero unmarked — a fresh install really did spend $0.00', () => {
    state.db = emptyStore();
    const out = sessionHistory.summary();
    expect(out.totals.sessions).toBe(0);
    expect(out.totals.costUSD).toBe(0);
    expect(out.unavailable).toBeUndefined();
  });
});

describe('sessionHistory.recent — an unreadable store rejects, it does not answer []', () => {
  beforeEach(() => {
    state.db = null;
  });

  it('throws rather than claiming the history is empty', () => {
    expect(() => sessionHistory.recent()).toThrow(/database unavailable/);
  });

  it('returns [] when the store answers and truly has no rows', () => {
    state.db = emptyStore();
    expect(sessionHistory.recent()).toEqual([]);
  });
});
