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
