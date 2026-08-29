/**
 * The History pane is the surface with the widest gap between what the app
 * knows and what it shows: `session_history` holds every session the desktop
 * ever recorded, and until this wiring the pane listed them all as bare titles.
 *
 * Two things are pinned here.
 *
 * REAL NUMBERS. A row the daemon has forgotten — a transcript-only row, which
 * is most of a long history — still gets its recorded cost and tokens, because
 * the pane reads `analytics:recent` directly rather than relying on the daemon
 * join.
 *
 * THREE STATES, NEVER TWO. Unknown, unavailable and zero are different facts:
 * a row nobody recorded usage for shows NOTHING (not "$0.00"), and a store that
 * could not be read says so in prose (not a total of zero). The headless hub
 * makes the third case routine — it answers `analytics.summary` with a
 * well-formed all-zero payload carrying `unavailable: "headless"`.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/react';
import SessionsPane from '../src/panes/SessionsPane';
import type { SessionAnalytics } from '../src/hooks/useSessionAnalytics';
import type { RecentAgentSession } from '../../main/shared/ipcTypes';

const DIR = '/home/u/Work/demo';

vi.mock('../src/contexts/ConfigContext', () => ({
  useConfigContext: () => ({ config: { projects: { [DIR]: {} } } }),
}));

const transcripts = vi.hoisted(() => ({
  byDir: {} as Record<string, { sessionId: string; timestamp: string; summary: string }[]>,
}));
vi.mock('../src/hooks/useProjectSessions', () => ({
  useProjectSessions: () => ({ byDir: transcripts.byDir, loading: false }),
}));

const analytics = vi.hoisted(() => ({ value: null as unknown }));
vi.mock('../src/hooks/useSessionAnalytics', () => ({
  useSessionAnalytics: () => analytics.value,
}));

function setAnalytics(over: Partial<SessionAnalytics> = {}): void {
  analytics.value = {
    summary: null,
    bySessionId: {},
    unrecordedSessions: 0,
    unrecordedComplete: true,
    loading: false,
    unavailable: null,
    refresh: () => {},
    ...over,
  } satisfies SessionAnalytics;
}

const summary = (costUSD: number, sessions: number, tokens = 0) => ({
  totals: {
    sessions,
    costUSD,
    inputTokens: tokens,
    outputTokens: 0,
    toolCalls: 0,
    durationMs: 0,
    workflowRuns: 0,
  },
  byDay: [],
  byProject: [],
  byModel: [],
  byProvider: [],
});

const daemonRow = (over: Partial<RecentAgentSession> = {}): RecentAgentSession =>
  ({
    sessionId: 'daemon-1',
    cwd: DIR,
    provider: 'claude',
    name: '',
    title: 'a daemon row',
    model: 'claude-opus-4-8',
    updatedAt: Date.now(),
    archived: false,
    ...over,
  }) as RecentAgentSession;

beforeEach(() => {
  transcripts.byDir = {
    [DIR]: [
      { sessionId: 'has-usage', timestamp: '2026-08-20T10:00:00Z', summary: 'a costed session' },
      { sessionId: 'no-usage', timestamp: '2026-08-19T10:00:00Z', summary: 'an un-costed session' },
    ],
  };
  setAnalytics();
});
afterEach(cleanup);

describe('SessionsPane — the history DB reaches the rows the daemon forgot', () => {
  it('renders a transcript-only row’s recorded cost and tokens', () => {
    setAnalytics({
      summary: summary(14892.67, 747),
      bySessionId: { 'has-usage': { costUSD: 2.5, billedTokens: 1000 } },
    });
    render(<SessionsPane sessions={[]} />);
    // No daemon row exists for this session at all — the figure can only have
    // come from the direct `analytics:recent` read.
    expect(screen.getByText(/\$2\.50/)).toBeInTheDocument();
  });

  it('shows nothing at all for a row nobody recorded usage for', () => {
    setAnalytics({ summary: summary(14892.67, 747), bySessionId: {} });
    render(<SessionsPane sessions={[]} />);
    expect(screen.getByText(/an un-costed session/)).toBeInTheDocument();
    // A stored zero is a placeholder, not a measurement: `cost_usd` is
    // `REAL DEFAULT 0` and never NULL, so "$0.00" would be a claim nobody made.
    expect(screen.queryByText(/\$0\.00/)).toBeNull();
  });

  it('lets the daemon row’s own figure win when it has one', () => {
    setAnalytics({ summary: summary(1, 1), bySessionId: {} });
    render(<SessionsPane sessions={[daemonRow({ costUSD: 7.25 })]} />);
    expect(screen.getByText(/\$7\.25/)).toBeInTheDocument();
  });

  it('shows the lifetime totals the store actually holds', () => {
    setAnalytics({ summary: summary(14892.67, 747, 17_500_000_000) });
    render(<SessionsPane sessions={[]} />);
    // fmtUSD is the app-wide formatter — no thousands separator by design.
    expect(screen.getByText(/\$14892\.67/)).toBeInTheDocument();
    expect(screen.getByText(/747 sessions/)).toBeInTheDocument();
  });
});

describe('SessionsPane — unavailable is not zero', () => {
  it('says the store could not be read instead of totalling it to $0.00', () => {
    // Exactly what an adopted headless hub answers.
    setAnalytics({ summary: null, unavailable: 'headless' });
    render(<SessionsPane sessions={[]} />);
    expect(screen.getByText(/unavailable here \(headless\)/)).toBeInTheDocument();
    expect(screen.queryByText(/\$0\.00/)).toBeNull();
    expect(screen.queryByText(/All time/)).toBeNull();
  });

  it('keeps a genuine zero from a reachable store as a measurement', () => {
    setAnalytics({ summary: summary(0, 0) });
    render(<SessionsPane sessions={[]} />);
    // A fresh install really has spent nothing, and that IS a reading.
    expect(screen.getByText(/\$0\.00/)).toBeInTheDocument();
    expect(screen.getByText(/0 sessions/)).toBeInTheDocument();
  });

  it('says it is still reading rather than showing a total it does not have', () => {
    setAnalytics({ loading: true });
    render(<SessionsPane sessions={[]} />);
    expect(screen.getByText(/Reading recorded usage/)).toBeInTheDocument();
    expect(screen.queryByText(/\$/)).toBeNull();
  });

  it('marks the un-costed count as a floor when the row read was capped', () => {
    setAnalytics({
      summary: summary(14892.67, 5000),
      unrecordedSessions: 2000,
      unrecordedComplete: false,
    });
    render(<SessionsPane sessions={[]} />);
    // The count covers the rows READ; the session count covers the store. Two
    // denominators, so the smaller number has to admit it is a lower bound.
    expect(screen.getByText(/at least 2000 with no usage recorded/)).toBeInTheDocument();
  });
});
