/**
 * Regression test: loadAgentsFromSession must select an active agent that
 * survived dedupe. dedupeBySessionId collapses same-session cards to the
 * smallest id; the active id was chosen from the RAW pre-dedupe list, so it
 * could point at a dropped card — leaving activeAgent undefined and the
 * workspace blank even though a usable card is present.
 */
import { describe, it, expect } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';

const mkAgent = (id: string, sessionId: string) =>
  ({
    id,
    name: id,
    cwd: '/x',
    sessionId,
    tabs: [{ id: `${id}-t`, title: id, panes: [{ id: `${id}-p`, type: 'claude', title: 'C' }], activePaneId: `${id}-p` }],
    activeTabId: `${id}-t`,
  }) as any;

describe('useAgentManager.loadAgentsFromSession — active id survives dedupe', () => {
  it('falls back to the surviving card when activeId is empty and the first raw card was dropped', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      // Two cards share session S1; dedupe keeps 'agent-aaa' (smaller id) and
      // drops 'agent-zzz'. The raw first card is 'agent-zzz'.
      result.current.loadAgentsFromSession([mkAgent('agent-zzz', 'S1'), mkAgent('agent-aaa', 'S1')], '');
    });
    expect(result.current.agents.some((a: any) => a.id === result.current.activeAgentId)).toBe(true);
    expect(result.current.activeAgent).toBeTruthy();
    expect(result.current.tabs.length).toBeGreaterThan(0);
  });

  it('maps an activeId pointing at a dropped card to the surviving same-session card', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-zzz', 'S1'), mkAgent('agent-aaa', 'S1')], 'agent-zzz');
    });
    expect(result.current.activeAgentId).toBe('agent-aaa');
    expect(result.current.activeAgent).toBeTruthy();
  });

  it('still honors an activeId that survives dedupe', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-aaa', 'S1'), mkAgent('agent-bbb', 'S2')], 'agent-bbb');
    });
    expect(result.current.activeAgentId).toBe('agent-bbb');
  });
});
