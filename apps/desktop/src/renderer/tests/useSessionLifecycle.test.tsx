/**
 * Regression: the autosave dedup hash must include each tab's activePaneId.
 * Switching the active pane inside a split tab changes nothing else in the
 * payload, so omitting activePaneId from the hash made the save dedup away and
 * the active-pane change never persisted.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useSessionLifecycle } from '../src/hooks/useSessionLifecycle';

let saveSession: ReturnType<typeof vi.fn>;

beforeEach(() => {
  saveSession = vi.fn().mockResolvedValue(undefined);
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    saveSession,
    listSessions: vi.fn().mockResolvedValue([]),
    getAllClaudeSessions: vi.fn().mockResolvedValue([]),
    onBeforeQuit: vi.fn().mockReturnValue(() => {}),
  };
});

function mkAgents(activePaneId: string) {
  return [
    {
      id: 'a1',
      name: 'A',
      cwd: '/x',
      sessionId: 's1',
      activeTabId: 't1',
      tabs: [
        {
          id: 't1',
          title: 'Tab',
          activePaneId,
          panes: [
            { id: 'p1', type: 'terminal' },
            { id: 'p2', type: 'editor' },
          ],
        },
      ],
    },
  ] as any;
}

function render(agents: any) {
  return renderHook(
    (props: { agents: any }) =>
      useSessionLifecycle({
        configLoaded: false, // skip startup effect
        agents: props.agents,
        activeAgentId: 'a1',
        loadAgentsFromSession: vi.fn(),
        reconcileAgents: vi.fn(),
        appCwdRef: { current: '/x' },
      }),
    { initialProps: { agents } },
  );
}

describe('useSessionLifecycle — autosave dedup hash', () => {
  it("persists a save when only a tab's activePaneId changed", () => {
    const { result, rerender } = render(mkAgents('p1'));

    act(() => result.current.setSessionPhase('active'));
    act(() => {
      void result.current.saveCurrentSession();
    });
    expect(saveSession).toHaveBeenCalledTimes(1);

    // Only activePaneId changes (p1 → p2) — everything else identical.
    rerender({ agents: mkAgents('p2') });
    act(() => {
      void result.current.saveCurrentSession();
    });

    // Must NOT be deduped away — the active-pane change has to persist.
    expect(saveSession).toHaveBeenCalledTimes(2);
  });

  it('still dedups an identical save', () => {
    const { result } = render(mkAgents('p1'));
    act(() => result.current.setSessionPhase('active'));
    act(() => {
      void result.current.saveCurrentSession();
    });
    act(() => {
      void result.current.saveCurrentSession();
    });
    expect(saveSession).toHaveBeenCalledTimes(1);
  });

  it('persists a save when only an agent model changed (a persisted field)', () => {
    // model/effort/permissionMode/cwd/skipPermissions are all written to disk
    // but were absent from the old dedup hash, so an edit confined to them
    // deduped away and only reached disk on a forced quit-save.
    function mkWithModel(model: string) {
      return [
        {
          id: 'a1',
          name: 'A',
          cwd: '/x',
          model,
          sessionId: 's1',
          activeTabId: 't1',
          tabs: [
            { id: 't1', title: 'Tab', activePaneId: 'p1', panes: [{ id: 'p1', type: 'terminal' }] },
          ],
        },
      ] as any;
    }
    const { result, rerender } = render(mkWithModel('sonnet'));
    act(() => result.current.setSessionPhase('active'));
    act(() => {
      void result.current.saveCurrentSession();
    });
    expect(saveSession).toHaveBeenCalledTimes(1);

    rerender({ agents: mkWithModel('opus') });
    act(() => {
      void result.current.saveCurrentSession();
    });
    expect(saveSession).toHaveBeenCalledTimes(2);
  });
});

describe('useSessionLifecycle — hardening', () => {
  it('persists a roster change within ~1s (debounced save), not only on the 30s tick', () => {
    vi.useFakeTimers();
    try {
      const { result, rerender } = render(mkAgents('p1'));
      act(() => result.current.setSessionPhase('active'));
      act(() => {
        vi.advanceTimersByTime(1100);
      });
      expect(saveSession).toHaveBeenCalledTimes(1);

      // A roster change (e.g. a terminate) re-arms the debounce.
      rerender({ agents: mkAgents('p2') });
      act(() => {
        vi.advanceTimersByTime(1100);
      });
      expect(saveSession).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('acks the quit-save so main can hold teardown for it', async () => {
    const notifyQuitSaved = vi.fn();
    let quitCb: (() => void) | undefined;
    (window as any).electronAPI = {
      ...(window as any).electronAPI,
      notifyQuitSaved,
      onBeforeQuit: vi.fn((cb: () => void) => {
        quitCb = cb;
        return () => {};
      }),
    };
    const { result } = render(mkAgents('p1'));
    act(() => result.current.setSessionPhase('active'));
    expect(quitCb).toBeDefined();
    await act(async () => {
      quitCb!();
    });
    expect(saveSession).toHaveBeenCalled();
    expect(notifyQuitSaved).toHaveBeenCalled();
  });

  it('acks the quit even when the save fails (a failure must not hang the quit)', async () => {
    saveSession.mockRejectedValueOnce(new Error('disk full'));
    const notifyQuitSaved = vi.fn();
    let quitCb: (() => void) | undefined;
    (window as any).electronAPI = {
      ...(window as any).electronAPI,
      notifyQuitSaved,
      onBeforeQuit: vi.fn((cb: () => void) => {
        quitCb = cb;
        return () => {};
      }),
    };
    const { result } = render(mkAgents('p1'));
    act(() => result.current.setSessionPhase('active'));
    await act(async () => {
      quitCb!();
    });
    expect(notifyQuitSaved).toHaveBeenCalled();
  });
});

describe('useSessionLifecycle — implicit-session boot', () => {
  it('boot restores the most recent session and reconciles dead sessions against the daemon', async () => {
    const reconcileAgents = vi.fn();
    const loadAgentsFromSession = vi.fn();
    (window as any).electronAPI = {
      ...(window as any).electronAPI,
      listSessions: vi
        .fn()
        .mockResolvedValue([{ name: 'S', filename: 's.yaml', timestamp: '', paneCount: 0 }]),
      loadSession: vi.fn().mockResolvedValue({ name: 'S', agents: [], activeAgentId: '' }),
      // Reconciliation asks claudemon itself for the live ids (the renderer
      // snapshot store is empty at boot) and auto-respawns the dead agents.
      listLiveClaudeSessionIds: vi.fn().mockResolvedValue(['live-1']),
    };
    const { result } = renderHook(() =>
      useSessionLifecycle({
        configLoaded: true,
        agents: [],
        activeAgentId: '',
        loadAgentsFromSession,
        reconcileAgents,
        appCwdRef: { current: '/x' },
      }),
    );
    await act(async () => {});
    expect((window as any).electronAPI.loadSession).toHaveBeenCalledWith('s.yaml');
    expect(loadAgentsFromSession).toHaveBeenCalledTimes(1);
    expect(result.current.sessionPhase).toBe('active');

    await waitFor(() => {
      expect(reconcileAgents).toHaveBeenCalledWith(new Set(['live-1']), { respawnStopped: true });
    });
    expect(reconcileAgents).toHaveBeenCalledTimes(1);
  });

  it('boot with no saved sessions starts fresh without loading anything', async () => {
    const loadAgentsFromSession = vi.fn();
    const loadSession = vi.fn();
    (window as any).electronAPI = {
      ...(window as any).electronAPI,
      listSessions: vi.fn().mockResolvedValue([]),
      loadSession,
    };
    const { result } = renderHook(() =>
      useSessionLifecycle({
        configLoaded: true,
        agents: [],
        activeAgentId: '',
        loadAgentsFromSession,
        reconcileAgents: vi.fn(),
        appCwdRef: { current: '/x' },
      }),
    );
    await act(async () => {});
    expect(loadSession).not.toHaveBeenCalled();
    expect(loadAgentsFromSession).not.toHaveBeenCalled();
    expect(result.current.sessionPhase).toBe('active');
  });
});
