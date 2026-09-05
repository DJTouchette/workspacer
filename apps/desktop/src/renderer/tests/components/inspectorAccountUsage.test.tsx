import React from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { InspectorCard } from '../../src/components/claude/InspectorCard';
import { refreshUsageReport, __resetUsageReportCache } from '../../src/hooks/useUsageReport';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';
import type { UsageReportAccount, UsageReportWire } from '../../../main/shared/usageReport';

/**
 * The Inspector's Usage tab now shows TWO different kinds of number: what this
 * session spent (context, tokens, cost) and the account allowance it spends
 * from. The second one is shared with every other session on the same login, so
 * the tests that matter are about attribution and labelling, not layout:
 *
 *  - it draws the row THIS session's identity names, and no other login's;
 *  - it says so when it cannot tell, instead of picking;
 *  - it reads the Overview's report through the Overview's shared poll and
 *    mapper, so the two surfaces cannot report different figures or a different
 *    verdict for one account;
 *  - and against a backend that has no report it renders exactly what the tab
 *    rendered before this existed.
 */

const now = 1_800_000_000;
const WORK_ROOT = '/home/u/.claude/accounts/work';
const WORK_TRANSCRIPT = `${WORK_ROOT}/projects/p/t.jsonl`;
const DEFAULT_TRANSCRIPT = '/home/u/.claude/projects/p/t.jsonl';

function acct(account: string | null, used: number, expected?: number): UsageReportAccount {
  return {
    account,
    label: account?.split('/').pop() || 'default',
    source: 'disk',
    observed_at: now - 300,
    fresh: null,
    windows: {
      five_hour: {
        used_percent: { state: 'ok', value: used },
        resets_at: now + 9000,
        window_minutes: 300,
        is_current: true,
        pace: {
          known: expected !== undefined,
          state: 'ahead',
          usedPct: used,
          expectedPct: expected,
          curve: 'calendar',
        },
      },
    },
  };
}

/** Two Claude logins on one machine — the case a session surface has to get
 *  right, and the reason "the freshest reading" is not an answer. */
function fixture(accounts: UsageReportAccount[] = [acct('', 10, 52), acct(WORK_ROOT, 70, 52)]) {
  return {
    evaluated_at: now,
    valid_until: now + 60,
    providers: [{ provider: 'claude', accounts }],
  } as UsageReportWire;
}

const snapshot = (overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot =>
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
    lastActivity: now * 1000,
    totalToolCalls: 3,
    provider: 'claude',
    transcriptPath: WORK_TRANSCRIPT,
    usage: {
      model: 'claude-opus-5',
      contextTokens: 50_000,
      contextLimit: 200_000,
      totalInputTokens: 1200,
      totalOutputTokens: 800,
      costUSD: 0.42,
    },
    ...overrides,
  }) as unknown as ClaudeSessionSnapshot;

const api = window.electronAPI as unknown as Record<string, unknown>;

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function mount(snap: ClaudeSessionSnapshot | null = snapshot()) {
  vi.useFakeTimers();
  vi.setSystemTime(now * 1000);
  return render(<InspectorCard snapshot={snap} initialTab="usage" />);
}

afterEach(() => {
  cleanup();
  __resetUsageReportCache();
  delete api.usageReport;
  vi.useRealTimers();
});

describe('the Inspector’s account allowance', () => {
  it('draws this session’s own login, paced, and switches with the session', async () => {
    api.usageReport = vi.fn().mockResolvedValue(fixture());
    const view = mount();
    await flush();

    // The work login's numbers, through the SHARED mapper: the same word and
    // the same error colour the Overview gives 70% against 52% expected.
    expect(screen.getByText('70%')).toBeInTheDocument();
    expect(screen.getByText('above pace')).toHaveStyle({ color: 'var(--wks-error)' });
    expect(screen.getByTestId('usage-consumed')).toHaveStyle({ background: 'var(--wks-error)' });
    expect((screen.getByTestId('usage-expected') as HTMLElement).style.left).toBe('52%');
    // …and NOT the other login's, which is the whole point.
    expect(screen.queryByText('10%')).not.toBeInTheDocument();

    // Inspecting the other session flips the row. Nothing is remembered from
    // the previous one.
    view.rerender(
      <InspectorCard
        snapshot={snapshot({ sessionId: 'sess-2', transcriptPath: DEFAULT_TRANSCRIPT })}
        initialTab="usage"
      />,
    );
    expect(screen.getByText('10%')).toBeInTheDocument();
    expect(screen.queryByText('70%')).not.toBeInTheDocument();
    expect(screen.queryByText('above pace')).not.toBeInTheDocument();
  });

  it('labels the allowance as the account’s, beside this session’s own figures', async () => {
    api.usageReport = vi.fn().mockResolvedValue(fixture());
    mount();
    await flush();
    expect(screen.getByText('Account allowance')).toBeInTheDocument();
    expect(
      screen.getByText(/shared with every other session on the same account/i),
    ).toBeInTheDocument();
    // The session's own metrics are untouched — this is an addition, not a
    // replacement, and the two must stay legible as different things.
    expect(screen.getByText('Input tokens')).toBeInTheDocument();
    expect(screen.getByText('Cost')).toBeInTheDocument();
    expect(screen.getByText('Context window')).toBeInTheDocument();
  });

  it('says it cannot tell rather than picking one of two logins', async () => {
    api.usageReport = vi.fn().mockResolvedValue(fixture());
    // A session with no transcript path yet: its login is genuinely unknown,
    // and `claudeAccountOf('')` would call it the default one.
    mount(snapshot({ transcriptPath: undefined }));
    await flush();
    expect(screen.getByText(/2 claude accounts are reported here/)).toBeInTheDocument();
    expect(screen.queryByText('70%')).not.toBeInTheDocument();
    expect(screen.queryByText('10%')).not.toBeInTheDocument();
    expect(screen.queryByTestId('usage-consumed')).not.toBeInTheDocument();
  });

  it('withholds a local reading from a session on a peer hub', async () => {
    api.usageReport = vi.fn().mockResolvedValue(fixture([acct('', 10, 52)]));
    // Federation blanks the transcript path, which reads as the DEFAULT login.
    mount(snapshot({ hub: 'studio', transcriptPath: '' }));
    await flush();
    expect(screen.getByText(/runs on hub/)).toBeInTheDocument();
    expect(screen.getByText('studio')).toBeInTheDocument();
    expect(screen.queryByText('10%')).not.toBeInTheDocument();
  });

  it('names the absence when the report knows nothing about this provider', async () => {
    api.usageReport = vi.fn().mockResolvedValue(fixture());
    mount(snapshot({ provider: 'codex', transcriptPath: undefined }));
    await flush();
    expect(screen.getByText(/No codex account allowance was reported/)).toBeInTheDocument();
  });

  // The currency rule: a percentage from a window whose reset has passed is
  // real history and a false present. The heading must not stand over nothing.
  it('drops a rolled-over window instead of drawing it', async () => {
    const stale = fixture([acct(WORK_ROOT, 70, 52)]);
    stale.providers![0].accounts![0].windows!.five_hour!.resets_at = now - 86_400;
    api.usageReport = vi.fn().mockResolvedValue(stale);
    mount();
    await flush();
    expect(screen.getByText(/No claude allowance window is currently running/)).toBeInTheDocument();
    expect(screen.queryByText('70%')).not.toBeInTheDocument();
  });

  it('shows an aged observation as stale rather than as a pace', async () => {
    const aged = fixture([acct(WORK_ROOT, 70, 52)]);
    aged.providers![0].accounts![0].fresh = false;
    api.usageReport = vi.fn().mockResolvedValue(aged);
    mount();
    await flush();
    expect(screen.getByText('Stale · pace unavailable')).toBeInTheDocument();
    expect(screen.getByText('70%')).toBeInTheDocument();
    expect(screen.queryByTestId('usage-expected')).not.toBeInTheDocument();
  });

  // An older hub answers `null`, and a backend that predates the report has no
  // method at all. Either way the tab is exactly what it was.
  it.each([
    ['a backend with no usage report', undefined],
    ['a hub that answers null', null],
  ])('adds nothing against %s', async (_name, answer) => {
    if (answer === null) api.usageReport = vi.fn().mockResolvedValue(null);
    mount();
    await flush();
    expect(screen.queryByTestId('session-account-usage')).not.toBeInTheDocument();
    expect(screen.queryByText('Account allowance')).not.toBeInTheDocument();
    // The existing metrics survive the absence.
    expect(screen.getByText('Input tokens')).toBeInTheDocument();
    expect(screen.getByText('Context window')).toBeInTheDocument();
  });

  // One report and one poll for the whole app: the hook's subscriber set is
  // what keeps a second Inspector (or the Overview beside it) from doubling the
  // loopback traffic, and leaving the tab has to give the subscription back.
  it('shares the Overview’s single poll and releases it with the tab', async () => {
    const fetcher = vi.fn().mockResolvedValue(fixture());
    api.usageReport = fetcher;
    const view = mount();
    render(<InspectorCard snapshot={snapshot()} initialTab="usage" />);
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(screen.getAllByText('70%')).toHaveLength(2);

    // The minute poll, still one request for both cards.
    await act(async () => {
      vi.advanceTimersByTime(60_000);
    });
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(2);

    // Switching to another tab unmounts the block; the last subscriber leaving
    // must stop both of the hook's timers, not leak one per Inspector opened.
    cleanup();
    const solo = render(<InspectorCard snapshot={snapshot()} initialTab="usage" />);
    await flush();
    fireEvent.click(screen.getByLabelText('Files'));
    expect(screen.queryByTestId('session-account-usage')).not.toBeInTheDocument();
    expect(vi.getTimerCount()).toBe(0);
    solo.unmount();
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  // The Settings "Usage schedule" save forces a re-read (refreshUsageReport).
  // The Inspector rides the same cache, so the new curve has to land here too —
  // a setting that only moves the Overview's tick is a setting that half works.
  it('follows a usage-schedule save through the shared cache', async () => {
    const before = fixture([acct(WORK_ROOT, 70, 52)]);
    const after = fixture([acct(WORK_ROOT, 70, 91)]);
    after.evaluated_at = now + 60;
    after.valid_until = now + 120;
    api.usageReport = vi.fn().mockResolvedValueOnce(before).mockResolvedValueOnce(after);
    mount();
    await flush();
    const tick = () => (screen.getByTestId('usage-expected') as HTMLElement).style.left;
    expect(tick()).toBe('52%');
    await act(async () => {
      vi.setSystemTime((now + 60) * 1000);
      await refreshUsageReport();
    });
    expect(tick()).toBe('91%');
  });
});
