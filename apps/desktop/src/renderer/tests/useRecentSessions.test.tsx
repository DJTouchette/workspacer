/**
 * The sidebar's RECENT list polls every 60s, so a terminated agent's session —
 * which the daemon flips to a resumable Stopped row only after teardown —
 * could stay invisible for up to a minute. requestRecentSessionsRefresh()
 * (fired by terminateAgent) must trigger a refetch burst that retries past
 * the daemon's teardown window instead of fetching once and missing the flip.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useRecentSessions } from '../src/hooks/useRecentSessions';
import { requestRecentSessionsRefresh } from '../src/lib/watchBus';

const row = (sessionId: string, mode: string) => ({
  sessionId,
  provider: 'claude',
  cwd: '/x',
  mode,
  transport: 'pty',
  archived: false,
  updatedAt: 1,
  startedAt: 1,
  name: '',
  title: '',
  model: '',
  costUSD: 0,
});

describe('useRecentSessions — refresh burst', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('refetches on requestRecentSessionsRefresh and retries until the daemon reports the flip', async () => {
    // First (mount) fetch sees the session live; the burst's early retries
    // still see it live; only the later retry sees the Stopped row.
    const list = vi
      .fn()
      .mockResolvedValueOnce([row('S1', 'responding')])
      .mockResolvedValueOnce([row('S1', 'responding')])
      .mockResolvedValue([row('S1', 'stopped')]);
    (window.electronAPI.listRecentAgentSessions as any) = list;

    const { result } = renderHook(() => useRecentSessions());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(list).toHaveBeenCalledTimes(1);
    expect(result.current[0]?.mode).toBe('responding');

    act(() => {
      requestRecentSessionsRefresh();
    });
    // Burst schedules 0s/2s/5s/10s retries — well inside the 60s poll gap.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(11_000);
    });
    expect(list.mock.calls.length).toBeGreaterThanOrEqual(4);
    expect(result.current[0]?.mode).toBe('stopped');
  });

  it('keeps the regular 60s poll alive after a burst', async () => {
    const list = vi.fn().mockResolvedValue([]);
    (window.electronAPI.listRecentAgentSessions as any) = list;

    renderHook(() => useRecentSessions());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    act(() => {
      requestRecentSessionsRefresh();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(11_000);
    });
    const afterBurst = list.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(list.mock.calls.length).toBeGreaterThan(afterBurst);
  });
});
