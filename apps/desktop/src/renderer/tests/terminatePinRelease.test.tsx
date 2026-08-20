/**
 * Closing an agent releases its harpoon pin claim: terminateAgent reports
 * `agent:closed { sessionIds }` (live + resumable ids — the same id after a
 * respawn), and App unpins any matching entry in ui.pinnedAgentSessions.
 * Session-keyed, so the release is exact: closing one of two agents in the
 * SAME cwd never touches the other's pin (the ambiguity that killed the
 * cwd-keyed store).
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
  it('reports the closed agent\'s session ids, and only its own', async () => {
    const events: Array<{ sessionIds?: string[] }> = [];
    const onClosed = (e: Event) => events.push((e as CustomEvent).detail);
    window.addEventListener('agent:closed', onClosed);

    const hook = renderHook(() => useAgentManager());
    act(() =>
      hook.result.current.loadAgentsFromSession(
        // Two agents in the SAME cwd — the case that broke cwd-keyed pins.
        [seedAgent('agent-a', '/repo', 'sa'), seedAgent('agent-b', '/repo', 'sb')],
        'agent-a',
      ),
    );

    await act(async () => hook.result.current.terminateAgent('agent-a'));
    expect(events).toEqual([{ sessionIds: ['sa'] }]);

    await act(async () => hook.result.current.terminateAgent('agent-b'));
    expect(events[1]).toEqual({ sessionIds: ['sb'] });

    window.removeEventListener('agent:closed', onClosed);
    hook.unmount();
  });
});
