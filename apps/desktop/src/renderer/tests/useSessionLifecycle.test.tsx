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
    act(() => result.current.saveCurrentSession());
    expect(saveSession).toHaveBeenCalledTimes(1);

    // Only activePaneId changes (p1 → p2) — everything else identical.
    rerender({ agents: mkAgents('p2') });
    act(() => result.current.saveCurrentSession());

    // Must NOT be deduped away — the active-pane change has to persist.
    expect(saveSession).toHaveBeenCalledTimes(2);
  });

  it('still dedups an identical save', () => {
    const { result } = render(mkAgents('p1'));
    act(() => result.current.handleNewSession());
    act(() => result.current.saveCurrentSession());
    act(() => result.current.saveCurrentSession());
    expect(saveSession).toHaveBeenCalledTimes(1);
  });
});
