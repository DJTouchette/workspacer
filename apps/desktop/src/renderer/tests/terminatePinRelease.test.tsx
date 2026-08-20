/**
 * Closing an agent releases its harpoon pin claim (user request 2026-08-20):
 * terminateAgent reports `agent:closed { cwd, lastInCwd }`, and App unpins the
 * cwd only when it was the LAST agent there — a second agent in the same repo
 * keeps the slot, and a stopped-but-kept card (not closed) never fires this.
 */
import { describe, it, expect } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';

const seedAgent = (id: string, cwd: string, sessionId: string) =>
  ({
    id,
    name: id,
    cwd,
    sessionId,
    tabs: [
      {
        id: `${id}-t`,
        title: id,
        panes: [{ id: `${id}-p`, type: 'claude', title: 'C', attachSessionId: sessionId }],
        activePaneId: `${id}-p`,
      },
    ],
    activeTabId: `${id}-t`,
  }) as any;

describe('terminateAgent → agent:closed', () => {
  it('reports lastInCwd only when no other agent shares the cwd', async () => {
    const events: Array<{ cwd?: string; lastInCwd?: boolean }> = [];
    const onClosed = (e: Event) => events.push((e as CustomEvent).detail);
    window.addEventListener('agent:closed', onClosed);

    const hook = renderHook(() => useAgentManager());
    act(() =>
      hook.result.current.loadAgentsFromSession(
        [seedAgent('agent-a', '/repo', 'sa'), seedAgent('agent-b', '/repo', 'sb')],
        'agent-a',
      ),
    );

    await act(async () => hook.result.current.terminateAgent('agent-a'));
    expect(events).toEqual([{ cwd: '/repo', lastInCwd: false }]);

    await act(async () => hook.result.current.terminateAgent('agent-b'));
    expect(events[1]).toEqual({ cwd: '/repo', lastInCwd: true });

    window.removeEventListener('agent:closed', onClosed);
    hook.unmount();
  });
});
