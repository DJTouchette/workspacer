/**
 * Regression test for useLayoutSync version monotonicity.
 *
 * A live `layout.changed` broadcast can arrive and apply a newer document
 * before the initial `layoutGet()` read resolves (the read captured an older
 * snapshot). The hydrate handler must never regress the applied version, and a
 * subsequently-arriving older broadcast must not clobber the newer layout.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useLayoutSync } from '../src/hooks/useLayoutSync';

let layoutChangedCb: ((doc: any) => void) | null = null;
let resolveLayoutGet: ((doc: any) => void) | null = null;

function mkAgent(id: string) {
  return { id, name: id, cwd: '/x', sessionId: id, tabs: [], activeTabId: '' } as any;
}

beforeEach(() => {
  layoutChangedCb = null;
  resolveLayoutGet = null;
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    layoutGet: vi.fn().mockReturnValue(new Promise((r) => { resolveLayoutGet = r; })),
    layoutSet: vi.fn().mockResolvedValue({ version: 99 }),
    onLayoutChanged: vi.fn().mockImplementation((cb: any) => { layoutChangedCb = cb; return () => {}; }),
    onHubStatus: vi.fn().mockReturnValue(() => {}),
  };
});

function renderSync(load: ReturnType<typeof vi.fn>) {
  return renderHook(() =>
    useLayoutSync({
      agents: [],
      activeAgentId: '',
      loadAgentsFromSession: load,
      sessionPhase: 'active',
      setSessionPhase: vi.fn(),
      enabled: true,
      onHydration: vi.fn(),
    }),
  );
}

describe('useLayoutSync — version monotonicity', () => {
  it('never applies a broadcast older than one already applied', async () => {
    const load = vi.fn();
    renderSync(load);

    // A newer layout (v5) is broadcast before the initial read resolves.
    act(() => layoutChangedCb!({ version: 5, data: { agents: [mkAgent('v5')], activeAgentId: 'v5' } }));
    // The initial read resolves late carrying an OLDER version (v2).
    await act(async () => { resolveLayoutGet!({ version: 2, data: { agents: [mkAgent('v2')], activeAgentId: 'v2' } }); await Promise.resolve(); });
    // A v3 broadcast arrives — older than the v5 we already applied; must be ignored.
    act(() => layoutChangedCb!({ version: 3, data: { agents: [mkAgent('v3')], activeAgentId: 'v3' } }));

    const appliedIds = load.mock.calls.map((c) => c[0]?.[0]?.id);
    // v3 must never be applied (v5 is newer and already applied).
    expect(appliedIds).not.toContain('v3');
    // The stale v2 read must not clobber v5 either.
    expect(appliedIds).not.toContain('v2');
    // v5 is the layout that won.
    expect(appliedIds).toContain('v5');
  });

  it('adopts a real layout from the initial read when no broadcast preceded it', async () => {
    const load = vi.fn();
    renderSync(load);
    await act(async () => { resolveLayoutGet!({ version: 1, data: { agents: [mkAgent('a1')], activeAgentId: 'a1' } }); await Promise.resolve(); });
    expect(load.mock.calls.map((c) => c[0]?.[0]?.id)).toContain('a1');
  });
});
