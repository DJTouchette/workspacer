/**
 * Regression test: a terminated agent must never reappear.
 *
 * Termination races the daemon — the dying session keeps emitting updates
 * (teardown hooks, final statusline ticks) until it reports `ended`. Those
 * ticks re-promote the session into App's snapshot map, where the auto-adopt
 * effect re-creates a card for any "live" session no agent owns. The fix is a
 * tombstone set: terminateAgent marks the session id, and every promotion /
 * adoption path skips tombstoned ids. This pins the marking half — that an
 * explicit terminate always tombstones, including the close-last-tab and
 * close-last-pane paths that call terminateAgent directly.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';
import { wasSessionTerminated, resetTerminatedSessions } from '../src/lib/terminatedSessions';

const mkAgent = (id: string, sessionId: string) =>
  ({
    id,
    name: id,
    cwd: '/x',
    sessionId,
    tabs: [
      {
        id: `${id}-t`,
        title: id,
        panes: [{ id: `${id}-p`, type: 'claude', title: 'C' }],
        activePaneId: `${id}-p`,
      },
    ],
    activeTabId: `${id}-t`,
  }) as any;

describe('terminateAgent tombstones the session', () => {
  beforeEach(() => resetTerminatedSessions());

  it('marks the session id and drops the card', async () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-a', 'S1')], 'agent-a');
    });
    expect(wasSessionTerminated('S1')).toBe(false);

    await act(async () => {
      await result.current.terminateAgent(
        result.current.agents.find((a: any) => a.sessionId === 'S1')!.id,
      );
    });
    expect(result.current.agents.some((a: any) => a.sessionId === 'S1')).toBe(false);
    expect(wasSessionTerminated('S1')).toBe(true);
  });

  it('closing the last tab (which terminates) also tombstones', async () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-b', 'S2')], 'agent-b');
    });
    const tabId = result.current.agents.find((a: any) => a.sessionId === 'S2')!.tabs[0].id;
    await act(async () => {
      result.current.removeTab(tabId);
    });
    expect(wasSessionTerminated('S2')).toBe(true);
  });
});
