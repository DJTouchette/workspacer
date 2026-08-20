/**
 * tmux-style zoom / swap / cycle on the real agent manager (COMMAND_LAYER.md,
 * Phase 3). The zoom state machine's rule is the point under test: any
 * structural or focus mutation UNZOOMS — a zoom that survives a split, a
 * close, or a focus move renders one pane over a layout that no longer
 * matches it.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';

const seed = () =>
  ({
    id: 'agent-1',
    name: 'a1',
    cwd: '/x',
    sessionId: 's1',
    tabs: [
      {
        id: 't1',
        title: 'T',
        panes: [
          { id: 'p1', type: 'claude', title: 'C' },
          { id: 'p2', type: 'terminal', title: 'T1' },
          { id: 'p3', type: 'terminal', title: 'T2' },
        ],
        activePaneId: 'p2',
      },
    ],
    activeTabId: 't1',
  }) as any;

const activeTab = (result: { current: ReturnType<typeof useAgentManager> }) => {
  const agent = result.current.agents.find((a) => a.id === 'agent-1')!;
  return agent.tabs[0];
};

let hook: { result: { current: ReturnType<typeof useAgentManager> } };

beforeEach(() => {
  hook = renderHook(() => useAgentManager());
  act(() => hook.result.current.loadAgentsFromSession([seed()], 'agent-1'));
});

describe('zoom / swap / cycle', () => {
  it('toggles zoom on the active pane and off again', () => {
    act(() => hook.result.current.toggleZoomPane());
    expect(activeTab(hook.result).zoomedPaneId).toBe('p2');
    act(() => hook.result.current.toggleZoomPane());
    expect(activeTab(hook.result).zoomedPaneId).toBeUndefined();
  });

  it('never zooms a single-pane tab', () => {
    act(() =>
      hook.result.current.loadAgentsFromSession(
        [
          {
            ...seed(),
            tabs: [
              {
                id: 't1',
                title: 'T',
                panes: [{ id: 'p1', type: 'claude', title: 'C' }],
                activePaneId: 'p1',
              },
            ],
          },
        ],
        'agent-1',
      ),
    );
    act(() => hook.result.current.toggleZoomPane());
    expect(activeTab(hook.result).zoomedPaneId).toBeUndefined();
  });

  it('focus moves unzoom (tmux select-pane), re-focusing the zoomed pane does not', () => {
    act(() => hook.result.current.toggleZoomPane());
    act(() => hook.result.current.setActivePane('t1', 'p2')); // same pane
    expect(activeTab(hook.result).zoomedPaneId).toBe('p2');
    act(() => hook.result.current.setActivePane('t1', 'p3')); // different pane
    expect(activeTab(hook.result).zoomedPaneId).toBeUndefined();
  });

  it('structural changes unzoom: split and close', () => {
    act(() => hook.result.current.toggleZoomPane());
    act(() => hook.result.current.splitTab('t1', 'terminal'));
    expect(activeTab(hook.result).zoomedPaneId).toBeUndefined();

    act(() => hook.result.current.setActivePane('t1', 'p2'));
    act(() => hook.result.current.toggleZoomPane());
    act(() => hook.result.current.removePane('t1', 'p1'));
    expect(activeTab(hook.result).zoomedPaneId).toBeUndefined();
    expect(activeTab(hook.result).panes.map((p: { id: string }) => p.id)).not.toContain('p1');
  });

  it('swapPane exchanges grid neighbours and clamps at the edges', () => {
    act(() => hook.result.current.swapPane('left'));
    expect(activeTab(hook.result).panes.map((p: { id: string }) => p.id)).toEqual([
      'p2',
      'p1',
      'p3',
    ]);
    expect(activeTab(hook.result).activePaneId).toBe('p2'); // focus rides the pane
    act(() => hook.result.current.swapPane('left')); // p2 now first — no-op
    expect(activeTab(hook.result).panes.map((p: { id: string }) => p.id)).toEqual([
      'p2',
      'p1',
      'p3',
    ]);
  });

  it('cyclePane wraps through tiling order', () => {
    act(() => hook.result.current.cyclePane()); // p2 → p3
    expect(activeTab(hook.result).activePaneId).toBe('p3');
    act(() => hook.result.current.cyclePane()); // p3 → p1 (wrap)
    expect(activeTab(hook.result).activePaneId).toBe('p1');
  });
});
