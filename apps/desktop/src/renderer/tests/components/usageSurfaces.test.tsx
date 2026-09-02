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
import { describe, it, expect, afterEach, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, cleanup, within, waitFor } from '@testing-library/react';
import React from 'react';
import { SessionStatusBar } from '../../src/components/claude/SessionStatusBar';
import { InspectorCard } from '../../src/components/claude/InspectorCard';
import { RateLimitCard } from '../../src/panes/OverviewPane';
import { __resetUsageReportCache, USAGE_REPORT_REFRESH_MS } from '../../src/hooks/useUsageReport';
import type { UsageReportWire } from '../../../main/shared/usageReport';
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

/**
 * THE COLD START.
 *
 * Every window this card draws used to come from a live session's status line,
 * so with nothing running it rendered null — no matter how good the daemon's
 * own reading was. claudemon answers GET /usage/report from disk and its
 * account poller with zero sessions; these pin that the card reads it, that it
 * never lets the report speak over a live line, and that it refuses to draw a
 * window the report says is not running.
 */
describe('the Overview usage card with no session at all', () => {
  const api = window.electronAPI as unknown as Record<string, unknown>;

  /** One provider/account row of claudemon's report. */
  const doc = (
    provider: string,
    account: string,
    windows: Record<string, unknown>,
  ): UsageReportWire => ({
    generated_at: NOW,
    providers: [
      {
        provider,
        accounts: [
          { account, label: account.split('/').pop() || 'default', is_default: false, windows },
        ],
      },
    ],
  });

  /** A window the report says is running. */
  const running = (pct: number | null, resetsIn: number, mins?: number) => ({
    used_percent:
      pct === null
        ? { state: 'unknown' as const, reason: 'no reading' }
        : { state: 'ok' as const, value: pct },
    resets_at: NOW + resetsIn,
    window_minutes: mins ?? null,
    is_current: true,
  });

  beforeEach(() => {
    __resetUsageReportCache();
    delete api.usageReport;
  });

  afterEach(() => {
    __resetUsageReportCache();
  });

  it('polls on a cadence the daemon can actually move under', () => {
    // The daemon re-polls an idle account every 15 minutes, so a faster clock
    // here asks a question whose answer cannot have changed — and this is a
    // loopback round trip per tick per open window. One minute is the floor.
    expect(USAGE_REPORT_REFRESH_MS).toBeGreaterThanOrEqual(60_000);
  });

  it('renders the daemon’s reading with zero snapshots', async () => {
    const acct = '/home/u/.claude/accounts/coldboot';
    api.usageReport = vi.fn().mockResolvedValue(
      doc('claude', acct, {
        five_hour: running(18, 3 * 3600, 300),
        seven_day: running(91, 4 * 86400, 10080),
      }),
    );

    render(<RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="coldboot" />);

    // Nothing at first — this is exactly the old behaviour, and it is what the
    // report is fetched to replace.
    await waitFor(() => expect(screen.getByText('18%')).toBeInTheDocument());
    expect(screen.getByText('91%')).toBeInTheDocument();
    // …and it is a real card, not a bare readout: the detail dialog opens.
    expect(screen.getByLabelText('Show usage detail')).toBeInTheDocument();
  });

  it('lets a live status line win, and fills only what the line never carried', async () => {
    const account = 'livewins';
    const path = '/home/u/.claude/accounts/livewins/projects/p/t.jsonl';
    api.usageReport = vi.fn().mockResolvedValue(
      doc('claude', '/home/u/.claude/accounts/livewins', {
        // Contradicts the live line on 5h, and speaks for a window the live
        // line is silent about.
        five_hour: running(77, 3 * 3600, 300),
        seven_day: running(33, 4 * 86400, 10080),
      }),
    );

    render(
      <RateLimitCard
        snaps={[
          {
            sessionId: 'sess-live',
            provider: 'claude',
            transcriptPath: path,
            statusLine: {
              fiveHourPct: 11,
              fiveHourResetsAt: NOW + 3 * 3600,
              fiveHourWindowMins: 300,
              receivedAt: new Date().toISOString(),
            },
          },
        ]}
        provider="claude"
        title="Claude usage"
        account={account}
      />,
    );

    await waitFor(() => expect(screen.getByText('33%')).toBeInTheDocument());
    expect(screen.getByText('11%')).toBeInTheDocument();
    expect(screen.queryByText('77%')).not.toBeInTheDocument();
  });

  it('refuses a percentage from a window that is not running', async () => {
    const acct = '/home/u/.claude/accounts/rolledover';
    api.usageReport = vi.fn().mockResolvedValue(
      doc('claude', acct, {
        // Live on 2026-08-30: 67% against a reset two days in the past. Real
        // history, a false present.
        five_hour: {
          used_percent: { state: 'ok', value: 67 },
          resets_at: NOW - 2 * 86400,
          window_minutes: 300,
          is_current: false,
        },
        // A percentage with no reset time cannot say WHICH window it describes.
        monthly: {
          used_percent: { state: 'ok', value: 42 },
          resets_at: null,
          window_minutes: null,
          is_current: null,
        },
        seven_day: running(5, 4 * 86400, 10080),
      }),
    );

    render(
      <RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="rolledover" />,
    );

    await waitFor(() => expect(screen.getByText('5%')).toBeInTheDocument());
    // Neither lapsed figure is drawn, and neither leaves an empty meter behind:
    // the row simply is not there.
    expect(screen.queryByText('67%')).not.toBeInTheDocument();
    expect(screen.queryByText('42%')).not.toBeInTheDocument();
    expect(screen.queryByText('5h')).not.toBeInTheDocument();
    expect(screen.queryByText('Mo')).not.toBeInTheDocument();
    expect(screen.getByText('7d')).toBeInTheDocument();
  });

  /**
   * The currency rule has three independent clauses, and the obvious fixture
   * — a lapsed window that ALSO says is_current:false — satisfies all of them
   * at once, so any one of the three can be deleted with every test still
   * green. These separate them.
   */
  it('drops a window whose reset has passed even when the report calls it current', async () => {
    // is_current was computed at generated_at; this clock is the later one.
    api.usageReport = vi.fn().mockResolvedValue(
      doc('claude', '/home/u/.claude/accounts/staleflag', {
        five_hour: {
          used_percent: { state: 'ok', value: 67 },
          resets_at: NOW - 60,
          window_minutes: 300,
          is_current: true,
        },
        seven_day: running(5, 4 * 86400, 10080),
      }),
    );
    render(<RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="staleflag" />);
    await waitFor(() => expect(screen.getByText('7d')).toBeInTheDocument());
    expect(screen.queryByText('67%')).not.toBeInTheDocument();
    expect(screen.queryByText('5h')).not.toBeInTheDocument();
  });

  it('drops a window the report calls not current even when its reset is ahead', async () => {
    // The daemon knows something the timestamp does not say on its own.
    api.usageReport = vi.fn().mockResolvedValue(
      doc('claude', '/home/u/.claude/accounts/notcurrent', {
        five_hour: {
          used_percent: { state: 'ok', value: 67 },
          resets_at: NOW + 3600,
          window_minutes: 300,
          is_current: false,
        },
        seven_day: running(5, 4 * 86400, 10080),
      }),
    );
    render(
      <RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="notcurrent" />,
    );
    await waitFor(() => expect(screen.getByText('7d')).toBeInTheDocument());
    expect(screen.queryByText('67%')).not.toBeInTheDocument();
    expect(screen.queryByText('5h')).not.toBeInTheDocument();
  });

  it('never reads the number off a measurement that is not `ok`', async () => {
    // `unknown`/`unavailable` carry a REASON, not a reading — but the field is
    // one struct and a stale value can ride along in it. The window is still
    // running, so the row appears; the percentage must not.
    api.usageReport = vi.fn().mockResolvedValue(
      doc('claude', '/home/u/.claude/accounts/notok', {
        five_hour: {
          used_percent: { state: 'unknown', value: 99, reason: 'the poll failed' },
          resets_at: NOW + 3600,
          window_minutes: 300,
          is_current: true,
        },
      }),
    );
    render(<RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="notok" />);
    await waitFor(() => expect(screen.getByText('5h')).toBeInTheDocument());
    expect(screen.queryByText('99%')).not.toBeInTheDocument();
  });

  it('never lets the report’s unattributed bucket speak for the default login', async () => {
    // `account: null` is the daemon saying it could not attribute those
    // sessions to any login. Folding it into the default account's card would
    // present somebody else's windows — or nobody's — as yours.
    api.usageReport = vi.fn().mockResolvedValue({
      generated_at: NOW,
      providers: [
        {
          provider: 'claude',
          accounts: [
            {
              account: null,
              label: 'unattributed',
              is_default: false,
              windows: { five_hour: running(88, 3 * 3600, 300) },
            },
            { account: '', label: 'default', is_default: true, windows: {} },
          ],
        },
      ],
    } as UsageReportWire);

    const { container } = render(
      <RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="" />,
    );
    await waitFor(() => expect(api.usageReport).toHaveBeenCalled());
    expect(screen.queryByText('88%')).not.toBeInTheDocument();
    expect(container).toBeEmptyDOMElement();
  });

  it('renders nothing when the report has no running window for the account', async () => {
    // What `usage.pollOnBoot: false` looks like from here: the daemon never
    // polled the idle account, so the report carries a reason instead of a
    // reading. The card must go back to drawing nothing rather than a row of
    // empty meters.
    api.usageReport = vi.fn().mockResolvedValue(
      doc('claude', '/home/u/.claude/accounts/notpolled', {
        five_hour: {
          used_percent: { state: 'unknown', reason: 'no reading for this account' },
          resets_at: null,
          window_minutes: null,
          is_current: null,
        },
        seven_day: {
          used_percent: { state: 'unknown', reason: 'no reading for this account' },
          resets_at: null,
          window_minutes: null,
          is_current: null,
        },
      }),
    );

    const { container } = render(
      <RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="notpolled" />,
    );

    await waitFor(() => expect(api.usageReport).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByLabelText('Show usage detail')).not.toBeInTheDocument();
  });

  it('keeps the last good reading when a later read cannot answer', async () => {
    // null is "we could not ask" — the daemon restarting, a timeout — not "no
    // windows". Letting it clear the cache would blank every card on the
    // desktop for a transient failure, which is the same lie as rendering 0%.
    api.usageReport = vi
      .fn()
      .mockResolvedValueOnce(
        doc('claude', '/home/u/.claude/accounts/keeplast', {
          five_hour: running(18, 3 * 3600, 300),
        }),
      )
      .mockResolvedValue(null);

    const first = render(
      <RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="keeplast" />,
    );
    await waitFor(() => expect(within(first.container).getByText('18%')).toBeInTheDocument());

    // A second card mounting re-reads, and this time the daemon cannot answer.
    const second = render(
      <RateLimitCard snaps={[]} provider="claude" title="Claude usage" account="keeplast" />,
    );
    await waitFor(() => expect(api.usageReport).toHaveBeenCalledTimes(2));
    expect(within(first.container).getByText('18%')).toBeInTheDocument();
    expect(within(second.container).getByText('18%')).toBeInTheDocument();
  });

  it('renders nothing when the transport cannot reach the daemon at all', async () => {
    // The web/remote backends stub usageReport to null — "we could not ask".
    api.usageReport = vi.fn().mockResolvedValue(null);
    const { container } = render(<RateLimitCard snaps={[]} provider="codex" title="Codex usage" />);
    await waitFor(() => expect(api.usageReport).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it('answers a provider card that does not filter by account', async () => {
    // Codex has one account here and the card passes `account: undefined`.
    api.usageReport = vi
      .fn()
      .mockResolvedValue(
        doc('codex', '/home/u/.codex', { seven_day: running(4, 6 * 86400, 10080) }),
      );
    render(<RateLimitCard snaps={[]} provider="codex" title="Codex usage" />);
    await waitFor(() => expect(screen.getByText('4%')).toBeInTheDocument());
  });
});
