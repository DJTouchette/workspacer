/**
 * Regression: plugins must survive the hub boot race.
 *
 * On a cold hub start (first launch after an app update — normal relaunches
 * adopt the still-running hub) the mount-time plugin fetch lands before the
 * hub is up, and the boot-time plugin.loaded events fire before the renderer's
 * bus subscription attaches. The old code treated the failed fetch as an empty
 * registry and never refetched — plugins vanished from the palette until a
 * reinstall emitted a fresh plugin.* event. Fixed by: null result (hub
 * unreachable) → backoff retry, and a refetch on every hub `connected` status.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { usePlugins } from '../src/hooks/usePlugins';

const manifest = (id: string) => ({ id, name: id, panes: [], hotkeys: [] }) as any;

let listHubPlugins: ReturnType<typeof vi.fn>;
let statusCb: ((s: { connected: boolean }) => void) | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  listHubPlugins = vi.fn();
  statusCb = undefined;
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    listHubPlugins,
    onHubEvent: vi.fn().mockReturnValue(() => {}),
    onHubStatus: vi.fn((cb: (s: { connected: boolean }) => void) => {
      statusCb = cb;
      return () => {};
    }),
  };
});

afterEach(() => {
  vi.useRealTimers();
});

describe('usePlugins — hub boot race', () => {
  it('retries an unreachable hub (null) instead of settling on empty', async () => {
    listHubPlugins.mockResolvedValueOnce(null).mockResolvedValue([manifest('acme.tool')]);
    const { result } = renderHook(() => usePlugins());
    await act(async () => {}); // flush the mount fetch (null → retry armed)
    expect(result.current.plugins).toEqual([]);

    await act(async () => {
      vi.advanceTimersByTime(1100); // first backoff tick
    });
    expect(listHubPlugins).toHaveBeenCalledTimes(2);
    expect(result.current.plugins.map((p) => p.id)).toEqual(['acme.tool']);
  });

  it('an empty list is accepted as-is (no retry loop for a bare registry)', async () => {
    listHubPlugins.mockResolvedValue([]);
    renderHook(() => usePlugins());
    await act(async () => {});
    await act(async () => {
      vi.advanceTimersByTime(30_000);
    });
    expect(listHubPlugins).toHaveBeenCalledTimes(1);
  });

  it('refetches when the hub reports connected — including the first connect', async () => {
    listHubPlugins.mockResolvedValueOnce(null).mockResolvedValue([manifest('acme.tool')]);
    const { result } = renderHook(() => usePlugins());
    await act(async () => {});
    expect(result.current.plugins).toEqual([]);

    // Hub finishes booting and the bus connects — no need to wait for backoff.
    await act(async () => {
      statusCb?.({ connected: true });
    });
    expect(result.current.plugins.map((p) => p.id)).toEqual(['acme.tool']);
  });
});
