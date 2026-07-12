/**
 * Regression (#5): adopting a remote layout must not immediately re-push a
 * mutated copy back to the hub. loadAgentsFromSession normalizes the incoming
 * layout (dedupe + inject the global Overview workspace + resolve the active
 * id), so the stored state differs from the raw doc. The echo-breaker was set
 * from the RAW doc, so the very next push-effect run saw the normalized state
 * as a "local change" and wrote it back — write amplification / echo loop.
 *
 * This wires the REAL useAgentManager into useLayoutSync so the real transform
 * runs, and asserts the hub is not written to after a pure adoption.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useState } from 'react';
import { useLayoutSync } from '../src/hooks/useLayoutSync';
import { useAgentManager } from '../src/hooks/useAgentManager';

let layoutSet: ReturnType<typeof vi.fn>;
let resolveGet: ((doc: any) => void) | null;

beforeEach(() => {
  layoutSet = vi.fn().mockResolvedValue({ version: 6 });
  resolveGet = null;
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    layoutGet: vi.fn().mockReturnValue(
      new Promise((r) => {
        resolveGet = r;
      }),
    ),
    layoutSet,
    onLayoutChanged: vi.fn().mockReturnValue(() => {}),
    onHubStatus: vi.fn().mockReturnValue(() => {}),
  };
});

// A remote layout that carries a real agent but NO global workspace — so the
// local normalization injects one, making stored state ≠ the raw doc.
const remoteAgent = {
  id: 'r1',
  name: 'Remote',
  cwd: '/repo',
  sessionId: 'sess-r1',
  activeTabId: 't1',
  tabs: [{ id: 't1', title: 'T', activePaneId: 'p1', panes: [{ id: 'p1', type: 'terminal' }] }],
};

function useCombined() {
  const mgr = useAgentManager();
  const [phase, setPhase] = useState<'loading' | 'picker' | 'active'>('loading');
  useLayoutSync({
    agents: mgr.agents,
    activeAgentId: mgr.activeAgentId,
    loadAgentsFromSession: mgr.loadAgentsFromSession,
    sessionPhase: phase,
    setSessionPhase: setPhase,
    enabled: true,
    adoptSharedLayout: true,
    onHydration: () => {},
  });
  return { mgr, phase };
}

describe('useLayoutSync — adopting a remote layout (#5)', () => {
  it('does not push the normalized layout back to the hub after adoption', async () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useCombined());

    // Hub returns a populated layout with no global workspace.
    await act(async () => {
      resolveGet!({ version: 5, data: { agents: [remoteAgent], activeAgentId: 'r1' } });
      await Promise.resolve();
    });

    // The remote agent was adopted and a global Overview workspace injected.
    expect(result.current.mgr.agents.some((a) => a.id === 'r1')).toBe(true);
    expect(result.current.mgr.agents.some((a) => a.global)).toBe(true);

    // Let the push debounce (250ms) elapse.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    // A pure adoption must not write anything back to the hub.
    expect(layoutSet).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('dedupes same-session agents while adopting a shared layout', async () => {
    const { result } = renderHook(() => useCombined());
    const duplicate = {
      ...remoteAgent,
      id: 'r1-duplicate',
      name: 'Remote duplicate',
    };

    await act(async () => {
      resolveGet!({
        version: 5,
        data: { agents: [remoteAgent, duplicate], activeAgentId: 'r1-duplicate' },
      });
      await Promise.resolve();
    });

    const realAgents = result.current.mgr.agents.filter((a) => !a.global);
    expect(realAgents.filter((a) => a.sessionId === 'sess-r1')).toHaveLength(1);
    expect(result.current.mgr.activeAgentId).toBe('r1');
  });
});
