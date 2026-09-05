import React from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { OverviewUsageCards } from '../../src/panes/OverviewPane';
import { UsageReportCard } from '../../src/components/UsageReportCard';
import {
  useUsageReport,
  refreshUsageReport,
  __resetUsageReportCache,
} from '../../src/hooks/useUsageReport';
import type { UsageReportWire } from '../../../main/shared/usageReport';
const now = 1_800_000_000;
function fixture(): UsageReportWire {
  return {
    evaluated_at: now,
    valid_until: now + 60,
    providers: [
      {
        provider: 'claude',
        accounts: ['', '/a/work', '/b/work', null].map((account) => ({
          account,
          label: 'work',
          source: 'disk',
          observed_at: now - 300,
          windows: {
            five_hour: {
              used_percent: { state: 'ok', value: 0 },
              resets_at: now + 9000,
              window_minutes: 300,
              pace: {
                known: true,
                state: 'on_track',
                usedPct: 0,
                expectedPct: 52,
                curve: 'calendar',
              },
            },
          },
        })),
      },
    ],
  };
}
function Cards() {
  const report = useUsageReport();
  return (
    <>
      {report?.providers?.flatMap((p) =>
        p.accounts?.map((a, i) => (
          <UsageReportCard key={i} report={report} provider={p.provider} account={a} />
        )),
      )}
    </>
  );
}
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
afterEach(() => {
  cleanup();
  __resetUsageReportCache();
  vi.useRealTimers();
});
describe('paced Overview accounts', () => {
  it('shows cold-start accounts, zero fill and truthful keyboard/click detail', () => {
    vi.useFakeTimers();
    vi.setSystemTime(now * 1000);
    const report = fixture();
    render(
      <>
        {report.providers![0].accounts!.map((a, i) => (
          <UsageReportCard key={i} report={report} provider="claude" account={a} />
        ))}
      </>,
    );
    expect(screen.getAllByTestId('usage-consumed')[0]).toHaveStyle({ width: '0%' });
    expect(screen.getAllByTestId('usage-expected')).toHaveLength(4);
    for (const [id, key] of [
      ['/a/work', 'Enter'],
      ['/b/work', ' '],
      ['Unattributed account', 'click'],
      ['Default account', 'Enter'],
    ]) {
      const card = screen.getByRole('button', { name: `Show usage detail: claude · ${id}` });
      if (key === 'click') fireEvent.click(card);
      else fireEvent.keyDown(card, { key });
      const dialog = screen.getByRole('dialog');
      expect(within(dialog).getByText(id)).toBeInTheDocument();
      fireEvent.click(within(dialog).getByLabelText('Close'));
    }
  });
  it('uses attention colour and explanatory text for ahead', () => {
    vi.useFakeTimers();
    vi.setSystemTime(now * 1000);
    const report = fixture();
    const a = report.providers![0].accounts![0];
    a.windows!.five_hour!.used_percent!.value = 70;
    a.windows!.five_hour!.pace!.usedPct = 70;
    render(<UsageReportCard report={report} account={a} provider="claude" />);
    expect(screen.getByText('ahead')).toHaveStyle({ color: 'var(--wks-warning)' });
    expect(screen.getByLabelText(/Spending faster than expected/)).toHaveAttribute(
      'title',
      expect.stringContaining('52% expected by now'),
    );
  });
  it('shares polling and expires pace/reset locally while a response is pending', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(now * 1000);
    const report = fixture();
    report.providers![0].accounts![0].windows!.five_hour!.resets_at = now + 2;
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(report)
      .mockImplementation(() => new Promise(() => {}));
    window.electronAPI.usageReport = fetcher;
    const view = render(
      <>
        <Cards />
        <Cards />
      </>,
    );
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(screen.getAllByTestId('usage-expected')).toHaveLength(8);
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(screen.getAllByTestId('usage-expected')).toHaveLength(6);
    await act(async () => {
      vi.advanceTimersByTime(58000);
    });
    await flush();
    expect(screen.queryAllByTestId('usage-expected')).toHaveLength(0);
    expect(fetcher).toHaveBeenCalledTimes(2);
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
  // The pacing SCHEDULE is hub-side, so a Settings save changes what the very
  // next usage.report computes. Two things have to hold or the setting reads as
  // inert: the forced re-read must not be swallowed by the poll already in
  // flight, and the older poll's reply must not land on top of the newer one
  // just because it finished second.
  it('re-reads on demand after a settings save, and an older reply cannot overwrite it', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(now * 1000);
    const before = fixture();
    const after = fixture();
    for (const a of after.providers![0].accounts!) a.windows!.five_hour!.pace!.expectedPct = 91;
    // The post-save sample is evaluated at the moment it is fetched, a minute
    // later, exactly as the hub would stamp it — otherwise the sixty-second
    // sample window would drop its tick and this test would be measuring the
    // validity guard instead of the refresh.
    after.evaluated_at = now + 60;
    after.valid_until = now + 120;

    // The poll's fetch is still open when the save happens; the forced one
    // resolves first with the NEW schedule's numbers.
    let releaseStale!: (r: UsageReportWire) => void;
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(before)
      .mockImplementationOnce(
        () =>
          new Promise<UsageReportWire>((r) => {
            releaseStale = r;
          }),
      )
      .mockResolvedValueOnce(after);
    window.electronAPI.usageReport = fetcher;

    // The tick's position IS the expected percentage the hub computed.
    const tick = () => (screen.getAllByTestId('usage-expected')[0] as HTMLElement).style.left;
    render(<Cards />);
    await flush();
    expect(tick()).toBe('52%');

    // The minute poll fires and hangs.
    await act(async () => {
      vi.advanceTimersByTime(60000);
    });
    expect(fetcher).toHaveBeenCalledTimes(2);

    // A save forces a third read even though one is in flight.
    await act(async () => {
      await refreshUsageReport();
    });
    expect(fetcher).toHaveBeenCalledTimes(3);
    expect(tick()).toBe('91%');

    // Now the hung poll answers with the PRE-save projection. It must be
    // dropped, not painted.
    await act(async () => {
      releaseStale(before);
    });
    await flush();
    expect(tick()).toBe('91%');
  });

  it('ages a failed transport and fences old responses when switching backend', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(now * 1000);
    window.electronAPI.usageReport = vi
      .fn()
      .mockResolvedValueOnce(fixture())
      .mockRejectedValue(new Error('offline'));
    const view = render(<Cards />);
    await flush();
    await act(async () => {
      vi.advanceTimersByTime(60000);
    });
    await flush();
    expect(screen.queryAllByTestId('usage-expected')).toHaveLength(0);
    expect(screen.getAllByText('Stale · pace unavailable')).toHaveLength(4);
    let resolve!: (r: UsageReportWire) => void;
    window.electronAPI.usageReport = vi.fn(
      () =>
        new Promise((r) => {
          resolve = r;
        }),
    );
    view.rerender(<Cards />);
    await flush();
    window.electronAPI.usageReport = vi.fn().mockResolvedValue({ providers: [] });
    view.rerender(<Cards />);
    await flush();
    await act(async () => {
      resolve(fixture());
    });
    await flush();
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });
});

it('the Overview uses whole canonical accounts despite conflicting and federated live readings', () => {
  vi.useFakeTimers();
  vi.setSystemTime(now * 1000);
  const report = fixture();
  const snaps = [
    {
      sessionId: 'remote',
      provider: 'claude',
      transcriptPath: '',
      statusLine: { fiveHourPct: 67, fiveHourResetsAt: now + 9000 },
    },
    {
      sessionId: 'local',
      provider: 'claude',
      transcriptPath: '/a/work/projects/p/t.jsonl',
      statusLine: { fiveHourPct: 99, fiveHourResetsAt: now + 18000 },
    },
  ];
  const view = render(<OverviewUsageCards usageReport={report} snaps={snaps} />);
  expect(screen.getAllByTestId('usage-consumed')).toHaveLength(4);
  expect(screen.queryByText('67%')).not.toBeInTheDocument();
  expect(screen.queryByText('99%')).not.toBeInTheDocument();
  const removed = structuredClone(report);
  for (const a of removed.providers![0].accounts!)
    a.windows!.five_hour!.used_percent = { state: 'unavailable' };
  view.rerender(<OverviewUsageCards usageReport={removed} snaps={snaps} />);
  expect(screen.queryAllByTestId('usage-consumed')).toHaveLength(0);
  expect(screen.queryAllByRole('button')).toHaveLength(0);
});
