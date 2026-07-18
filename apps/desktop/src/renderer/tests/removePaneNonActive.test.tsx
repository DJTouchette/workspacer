/**
 * Regression: removePane must close a pane that belongs to a NON-active agent
 * (bus/MCP command.close_pane can target a background agent's pane). The original
 * implementation mutated only the active agent, so closing a pane in a background
 * agent was a silent no-op.
 */
import { describe, it, expect } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';

const mkAgent = (id: string, sessionId: string, panes: any[]) =>
  ({
    id,
    name: id,
    cwd: '/x',
    sessionId,
    tabs: [{ id: `${id}-t`, title: id, panes, activePaneId: panes[0].id }],
    activeTabId: `${id}-t`,
  }) as any;

describe('useAgentManager.removePane — targets the owning agent', () => {
  it('closes a pane belonging to a non-active agent', () => {
    const { result } = renderHook(() => useAgentManager());
    const A = mkAgent('agent-a', 'SA', [{ id: 'a-p1', type: 'claude', title: 'C' }]);
    const B = mkAgent('agent-b', 'SB', [
      { id: 'b-p1', type: 'terminal', title: 'T1' },
      { id: 'b-p2', type: 'terminal', title: 'T2' },
    ]);
    act(() => {
      result.current.loadAgentsFromSession([A, B], 'agent-a');
    });
    expect(result.current.activeAgentId).toBe('agent-a');

    act(() => {
      result.current.removePane('agent-b-t', 'b-p2');
    });

    const agentB = result.current.agents.find((a: any) => a.id === 'agent-b');
    const tabB = agentB?.tabs.find((t: any) => t.id === 'agent-b-t');
    expect(tabB?.panes.map((p: any) => p.id)).toEqual(['b-p1']);
  });
});
