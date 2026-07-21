/**
 * Boot reconciliation trigger — the regression behind "agents spin on
 * Connecting… after a reboot": reconcile used to run only inside the LOCAL
 * session-restore branch, so a hub-adopted layout (the normal boot path since
 * the layout doc persists across reboots) never reconciled at all. It must now
 * fire whenever sessionPhase reaches 'active', whichever path got it there,
 * and retry while claudemon is still coming up (null = daemon unreachable).
 */
import { describe, it, expect, beforeEach, vi, type Mock } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useSessionLifecycle } from '../src/hooks/useSessionLifecycle';

const liveIds = window.electronAPI.listLiveClaudeSessionIds as Mock;

function makeOptions(over: Record<string, unknown> = {}) {
  return {
    configLoaded: false,
    agents: [],
    activeAgentId: '',
    loadAgentsFromSession: vi.fn(),
    reconcileAgents: vi.fn(),
    appCwdRef: { current: '/x' },
    ...over,
  } as any;
}

describe('useSessionLifecycle — daemon reconcile on phase active', () => {
  beforeEach(() => {
    liveIds.mockReset();
    (window.electronAPI.listSessions as Mock).mockResolvedValue([]);
  });

  it('reconciles (with auto-respawn) when the hub-adopted path flips the phase to active', async () => {
    liveIds.mockResolvedValue(['S1', 'S2']);
    const opts = makeOptions();
    const { result } = renderHook(() => useSessionLifecycle(opts));
    // Hub adoption path: useLayoutSync loads the layout and flips the phase
    // directly — the local restore (gated on configLoaded) never runs.
    act(() => result.current.setSessionPhase('active'));
    await waitFor(() => {
      expect(opts.reconcileAgents).toHaveBeenCalledWith(new Set(['S1', 'S2']), {
        respawnStopped: true,
      });
    });
  });

  it('retries while the daemon is unreachable (null) instead of marking agents dead', async () => {
    liveIds.mockResolvedValueOnce(null).mockResolvedValueOnce(['S1']);
    const opts = makeOptions();
    const { result } = renderHook(() => useSessionLifecycle(opts));
    act(() => result.current.setSessionPhase('active'));
    await waitFor(
      () => {
        expect(opts.reconcileAgents).toHaveBeenCalledTimes(1);
        expect(opts.reconcileAgents).toHaveBeenCalledWith(new Set(['S1']), {
          respawnStopped: true,
        });
      },
      { timeout: 3000 },
    );
    expect(liveIds).toHaveBeenCalledTimes(2);
  });

  it('also reconciles on the local-restore path (no saved sessions)', async () => {
    liveIds.mockResolvedValue([]);
    const opts = makeOptions({ configLoaded: true });
    renderHook(() => useSessionLifecycle(opts));
    await waitFor(() => {
      expect(opts.reconcileAgents).toHaveBeenCalledWith(new Set(), { respawnStopped: true });
    });
  });

  it('reconciles only once per boot, not on every phase re-render', async () => {
    liveIds.mockResolvedValue([]);
    const opts = makeOptions();
    const { result, rerender } = renderHook(() => useSessionLifecycle(opts));
    act(() => result.current.setSessionPhase('active'));
    await waitFor(() => expect(opts.reconcileAgents).toHaveBeenCalledTimes(1));
    rerender();
    act(() => result.current.setSessionPhase('active'));
    await new Promise((r) => setTimeout(r, 50));
    expect(opts.reconcileAgents).toHaveBeenCalledTimes(1);
  });
});
