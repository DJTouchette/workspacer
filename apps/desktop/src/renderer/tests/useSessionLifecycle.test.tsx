/**
 * Regression: the autosave dedup hash must include each tab's activePaneId.
 * Switching the active pane inside a split tab changes nothing else in the
 * payload, so omitting activePaneId from the hash made the save dedup away and
 * the active-pane change never persisted.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
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
        autoResume: false,
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
  it('persists a save when only a tab\'s activePaneId changed', () => {
    const { result, rerender } = render(mkAgents('p1'));

    act(() => result.current.handleNewSession()); // → phase 'active'
    act(() => { void result.current.saveCurrentSession(); });
    expect(saveSession).toHaveBeenCalledTimes(1);

    // Only activePaneId changes (p1 → p2) — everything else identical.
    rerender({ agents: mkAgents('p2') });
    act(() => { void result.current.saveCurrentSession(); });

    // Must NOT be deduped away — the active-pane change has to persist.
    expect(saveSession).toHaveBeenCalledTimes(2);
  });

  it('still dedups an identical save', () => {
    const { result } = render(mkAgents('p1'));
    act(() => result.current.handleNewSession());
    act(() => { void result.current.saveCurrentSession(); });
    act(() => { void result.current.saveCurrentSession(); });
    expect(saveSession).toHaveBeenCalledTimes(1);
  });
});

describe('useSessionLifecycle — hardening', () => {
  it('persists a roster change within ~1s (debounced save), not only on the 30s tick', () => {
    vi.useFakeTimers();
    try {
      const { result, rerender } = render(mkAgents('p1'));
      act(() => result.current.handleNewSession()); // → phase 'active'
      act(() => { vi.advanceTimersByTime(1100); });
      expect(saveSession).toHaveBeenCalledTimes(1);

      // A roster change (e.g. a terminate) re-arms the debounce.
      rerender({ agents: mkAgents('p2') });
      act(() => { vi.advanceTimersByTime(1100); });
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
      onBeforeQuit: vi.fn((cb: () => void) => { quitCb = cb; return () => {}; }),
    };
    const { result } = render(mkAgents('p1'));
    act(() => result.current.handleNewSession());
    expect(quitCb).toBeDefined();
    await act(async () => { quitCb!(); });
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
      onBeforeQuit: vi.fn((cb: () => void) => { quitCb = cb; return () => {}; }),
    };
    const { result } = render(mkAgents('p1'));
    act(() => result.current.handleNewSession());
    await act(async () => { quitCb!(); });
    expect(notifyQuitSaved).toHaveBeenCalled();
  });

  it('reconciles ended sessions as dead — they must not count as live on resume', async () => {
    const reconcileAgents = vi.fn();
    (window as any).electronAPI = {
      ...(window as any).electronAPI,
      loadSession: vi.fn().mockResolvedValue({ name: 'S', agents: [], activeAgentId: '' }),
      getAllClaudeSessions: vi.fn().mockResolvedValue([
        { sessionId: 'live-1', status: 'active' },
        { sessionId: 'dead-1', status: 'ended' },
      ]),
    };
    const { result } = renderHook(() =>
      useSessionLifecycle({
        configLoaded: false,
        autoResume: false,
        agents: [],
        activeAgentId: '',
        loadAgentsFromSession: vi.fn(),
        reconcileAgents,
        appCwdRef: { current: '/x' },
      }),
    );
    await act(async () => { result.current.handleResumeSession('s.yaml'); });
    expect(reconcileAgents).toHaveBeenCalledTimes(1);
    const liveSet = reconcileAgents.mock.calls[0][0] as Set<string>;
    expect(liveSet.has('live-1')).toBe(true);
    expect(liveSet.has('dead-1')).toBe(false);
  });
});
