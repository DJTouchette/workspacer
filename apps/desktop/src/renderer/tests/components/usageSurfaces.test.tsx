/**
 * WHICH surfaces let a reader open the usage detail, and what the windows say.
 *
 * The usage dialog shipped wired into exactly one surface: the agent pane's
 * status bar. The Overview card — the surface a reader calls "the dashboard",
 * and the one place usage is charted for a whole account — drew the same
 * windows with nothing to click, so the feature read as missing to anyone who
 * looked there first. This file pins the rule that came out of that: usage is
 * clickable wherever it is drawn, and the dialog it opens states how long each
 * window is.
 *
 * It also pins the two things that made the readout look broken at ordinary
 * numbers: a chip must appear well below the "close to the limit" threshold
 * (the report came in at 11% and 2%), and a backdrop click must actually close
 * the dialog — a portal bubbles through the REACT tree, so a dialog rendered
 * inside its own click target reopens itself on the way out.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup, within } from '@testing-library/react';
import React from 'react';
import { SessionStatusBar } from '../../src/components/claude/SessionStatusBar';
import { InspectorCard } from '../../src/components/claude/InspectorCard';
import { RateLimitCard } from '../../src/panes/OverviewPane';
import type { ClaudeSessionSnapshot, SessionStatusLine } from '../../src/types/claudeSession';

const NOW = Math.floor(Date.now() / 1000);

/** A Claude account comfortably inside both windows — the reported case. */
const statusLine = (): SessionStatusLine => ({
  modelDisplay: 'opus-5',
  contextUsedPct: 42,
  contextWindowSize: 200_000,
  totalInputTokens: 1_000,
  totalOutputTokens: 500,
  costUSD: 1.5,
  fiveHourPct: 11,
  fiveHourResetsAt: NOW + 3 * 3600,
  fiveHourWindowMins: 300,
  sevenDayPct: 2,
  sevenDayResetsAt: NOW + 4 * 86400,
  sevenDayWindowMins: 10080,
  receivedAt: new Date().toISOString(),
});

const snapshot = (): ClaudeSessionSnapshot =>
  ({
    sessionId: 'sess-1',
    cwd: '/repo',
    ptyId: 'sess-1',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    pendingQuestions: null,
    subagents: [],
    workflows: [],
    ambientState: 'idle',
    lastActivity: Date.now(),
    totalToolCalls: 0,
    usage: null,
    provider: 'claude',
    statusLine: statusLine(),
  }) as unknown as ClaudeSessionSnapshot;

/** Every surface that draws account rate-limit windows. A new one belongs in
 *  this list, and it has to earn its click target to get in. */
const SURFACES: Array<{ name: string; mount: () => void }> = [
  {
    name: 'the agent pane status bar',
    mount: () => render(<SessionStatusBar snapshot={snapshot()} cwd="/repo" />),
  },
  {
    name: "the inspector card's usage tab",
    mount: () => render(<InspectorCard snapshot={snapshot()} initialTab="usage" />),
  },
  {
    name: 'the Overview usage card',
    mount: () =>
      render(
        <RateLimitCard
          snaps={[{ sessionId: 'sess-1', provider: 'claude', statusLine: statusLine() }]}
          provider="claude"
          title="Claude usage"
        />,
      ),
  },
];

afterEach(cleanup);

describe('usage detail is reachable from every surface that draws windows', () => {
  for (const surface of SURFACES) {
    it(`opens from ${surface.name}`, () => {
      surface.mount();
      const target = screen.getByLabelText('Show usage detail');
      expect(target).toBeInTheDocument();

      fireEvent.click(target);
      const dialog = screen.getByRole('dialog', { name: 'Usage detail' });
      // The durations are the other half of the feature: the dialog must say
      // how long each window is, not just which slot it fills.
      expect(within(dialog).getByText(/5 hours/)).toBeInTheDocument();
      expect(within(dialog).getByText(/7 days/)).toBeInTheDocument();
      expect(within(dialog).getByText('11%')).toBeInTheDocument();
    });
  }
});

describe('the status bar chip at ordinary utilization', () => {
  it('shows the busiest window well below the warning threshold', () => {
    render(<SessionStatusBar snapshot={snapshot()} cwd="/repo" />);
    // 11% and 2% are nowhere near the 70% threshold; the bar used to render
    // nothing at all here, which left the reader with no target to click.
    expect(screen.getByText('11%')).toBeInTheDocument();
    expect(screen.getByText('5h')).toBeInTheDocument();
  });
});

describe('the Overview card speaks for an account, not a session', () => {
  it('omits the opening session’s own cost and tokens', () => {
    render(
      <RateLimitCard
        snaps={[{ sessionId: 'sess-1', provider: 'claude', statusLine: statusLine() }]}
        provider="claude"
        title="Claude usage"
      />,
    );
    fireEvent.click(screen.getByLabelText('Show usage detail'));
    const dialog = screen.getByRole('dialog', { name: 'Usage detail' });
    expect(within(dialog).getByText('Account limits')).toBeInTheDocument();
    expect(within(dialog).queryByText('Session cost')).not.toBeInTheDocument();
    expect(within(dialog).queryByText('This session')).not.toBeInTheDocument();
  });

  it('names the window length on each row for a reader who never opens it', () => {
    render(
      <RateLimitCard
        snaps={[{ sessionId: 'sess-1', provider: 'claude', statusLine: statusLine() }]}
        provider="claude"
        title="Claude usage"
      />,
    );
    expect(screen.getByTitle(/5-hour limit \(5 hours window\)/)).toBeInTheDocument();
    expect(screen.getByTitle(/7-day limit \(7 days window\)/)).toBeInTheDocument();
  });

  /**
   * THE BLANK BAR, renderer half.
   *
   * The daemon rebuilds a stream session's status line many times a minute
   * from wire data that carries `fiveHourResetsAt` and no `fiveHourPct`, and
   * stamps each rebuild with a fresh `receivedAt`. So the pct-less line is
   * always the NEWEST thing this card sees — and a whole-line "newest wins"
   * cache took it wholesale, leaving the card in place with an empty meter.
   *
   * The cache is per field: a line that does not mention a percentage says
   * nothing about it. Each test uses its own account so the module-level cache
   * cannot carry a reading between them.
   */
  it('keeps a remembered percentage when a newer line carries only a reset', () => {
    const snap = (sl: SessionStatusLine) => [
      {
        sessionId: 'sess-1',
        provider: 'claude',
        transcriptPath: '/home/u/.claude/accounts/displace/projects/p/t.jsonl',
        statusLine: sl,
      },
    ];
    const { rerender } = render(
      <RateLimitCard
        snaps={snap(statusLine())}
        provider="claude"
        title="Claude usage"
        account="displace"
      />,
    );
    expect(screen.getByText('11%')).toBeInTheDocument();

    // One second later, the rebuilt line: a reset time, no percentage.
    rerender(
      <RateLimitCard
        snaps={snap({
          fiveHourResetsAt: NOW + 3 * 3600,
          fiveHourWindowMins: 300,
          receivedAt: new Date(Date.now() + 1000).toISOString(),
        })}
        provider="claude"
        title="Claude usage"
        account="displace"
      />,
    );
    expect(screen.getByText('11%')).toBeInTheDocument();
    // …and the 7-day window, which the newer line does not mention at all,
    // must not vanish either.
    expect(screen.getByText('2%')).toBeInTheDocument();
  });

  it('prefers a fresher remembered reading over an older live one', () => {
    const account = 'older';
    const path = '/home/u/.claude/accounts/older/projects/p/t.jsonl';
    const { rerender } = render(
      <RateLimitCard
        snaps={[
          {
            sessionId: 'sess-1',
            provider: 'claude',
            transcriptPath: path,
            statusLine: statusLine(),
          },
        ]}
        provider="claude"
        title="Claude usage"
        account={account}
      />,
    );
    expect(screen.getByText('11%')).toBeInTheDocument();

    // A session whose statusLine has NO receivedAt sorts as timestamp 0 —
    // older than the cache. Neither branch of the old cache update fired for
    // that case, so the stale 99% was rendered over the fresher 11%.
    rerender(
      <RateLimitCard
        snaps={[
          {
            sessionId: 'sess-2',
            provider: 'claude',
            transcriptPath: path,
            statusLine: {
              fiveHourPct: 99,
              fiveHourResetsAt: NOW + 3600,
              fiveHourWindowMins: 300,
            },
          },
        ]}
        provider="claude"
        title="Claude usage"
        account={account}
      />,
    );
    expect(screen.getByText('11%')).toBeInTheDocument();
    expect(screen.queryByText('99%')).not.toBeInTheDocument();
  });

  it('closes on a backdrop click and stays closed', () => {
    render(
      <RateLimitCard
        snaps={[{ sessionId: 'sess-1', provider: 'claude', statusLine: statusLine() }]}
        provider="claude"
        title="Claude usage"
      />,
    );
    fireEvent.click(screen.getByLabelText('Show usage detail'));
    const dialog = screen.getByRole('dialog', { name: 'Usage detail' });
    // The backdrop is the dialog's parent; clicking it must not bubble back
    // into the card's own onClick and reopen what it just closed.
    fireEvent.click(dialog.parentElement as HTMLElement);
    expect(screen.queryByRole('dialog', { name: 'Usage detail' })).not.toBeInTheDocument();
  });
});
