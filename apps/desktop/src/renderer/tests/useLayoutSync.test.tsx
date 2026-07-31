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
    layoutGet: vi.fn().mockReturnValue(
      new Promise((r) => {
        resolveLayoutGet = r;
      }),
    ),
    layoutSet: vi.fn().mockResolvedValue({ version: 99 }),
    onLayoutChanged: vi.fn().mockImplementation((cb: any) => {
      layoutChangedCb = cb;
      return () => {};
    }),
    onHubStatus: vi.fn().mockReturnValue(() => {}),
  };
});

function renderSync(
  load: ReturnType<typeof vi.fn>,
  opts?: {
    adoptSharedLayout?: boolean;
    onHydration?: ReturnType<typeof vi.fn>;
    agents?: any[];
    activeAgentId?: string;
  },
) {
  return renderHook(() =>
    useLayoutSync({
      agents: opts?.agents ?? [],
      activeAgentId: opts?.activeAgentId ?? '',
      loadAgentsFromSession: load,
      sessionPhase: 'active',
      setSessionPhase: vi.fn(),
      enabled: true,
      adoptSharedLayout: opts?.adoptSharedLayout ?? true,
      onHydration: opts?.onHydration ?? vi.fn(),
    }),
  );
}

describe('useLayoutSync — version monotonicity', () => {
  it('never applies a broadcast older than one already applied', async () => {
    const load = vi.fn();
    renderSync(load);

    // A newer layout (v5) is broadcast before the initial read resolves.
    act(() =>
      layoutChangedCb!({ version: 5, data: { agents: [mkAgent('v5')], activeAgentId: 'v5' } }),
    );
    // The initial read resolves late carrying an OLDER version (v2).
    await act(async () => {
      resolveLayoutGet!({ version: 2, data: { agents: [mkAgent('v2')], activeAgentId: 'v2' } });
      await Promise.resolve();
    });
    // A v3 broadcast arrives — older than the v5 we already applied; must be ignored.
    act(() =>
      layoutChangedCb!({ version: 3, data: { agents: [mkAgent('v3')], activeAgentId: 'v3' } }),
    );

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
    await act(async () => {
      resolveLayoutGet!({ version: 1, data: { agents: [mkAgent('a1')], activeAgentId: 'a1' } });
      await Promise.resolve();
    });
    expect(load.mock.calls.map((c) => c[0]?.[0]?.id)).toContain('a1');
  });
});

describe('useLayoutSync — adoption gated on auto-resume', () => {
  // Regression: the hub persists its layout document across restarts, so an
  // unconditional adopt resurrected the previous run's panes on every boot.
  // With adoptSharedLayout off, a persisted layout must report 'empty' (so the
  // session picker runs) and must not be applied.
  it('reports empty and does not adopt a persisted layout when adoptSharedLayout is off', async () => {
    const load = vi.fn();
    const onHydration = vi.fn();
    renderSync(load, { adoptSharedLayout: false, onHydration });
    await act(async () => {
      resolveLayoutGet!({ version: 4, data: { agents: [mkAgent('old')], activeAgentId: 'old' } });
      await Promise.resolve();
    });
    expect(load).not.toHaveBeenCalled();
    expect(onHydration).toHaveBeenCalledWith('empty');
  });

  it('still applies live layout.changed broadcasts when adoptSharedLayout is off', async () => {
    const load = vi.fn();
    renderSync(load, { adoptSharedLayout: false });
    await act(async () => {
      resolveLayoutGet!({ version: 1, data: { agents: [mkAgent('old')], activeAgentId: 'old' } });
      await Promise.resolve();
    });
    // Another client writes while we're up — that's a live mirror, not a stale
    // boot-time document, and must still apply.
    act(() =>
      layoutChangedCb!({ version: 2, data: { agents: [mkAgent('live')], activeAgentId: 'live' } }),
    );
    expect(load.mock.calls.map((c) => c[0]?.[0]?.id)).toContain('live');
  });
});

/**
 * The layout document is the opposite of a secret: the hub persists it 0644
 * (the plugin token file is 0600) and broadcasts it to every connected client,
 * web and remote included. A plugin pane's URL carries that plugin's static bus
 * token as a query param, so publishing the URL verbatim shipped a live
 * credential to anything that could read the file or join the bus.
 */
describe('useLayoutSync — bus tokens never enter the shared document', () => {
  function pluginAgent() {
    return {
      id: 'a1',
      name: 'a1',
      cwd: '/x',
      sessionId: 'a1',
      activeTabId: 't1',
      tabs: [
        {
          id: 't1',
          title: 'Plugin',
          activePaneId: 'p1',
          panes: [
            {
              id: 'p1',
              type: 'plugin',
              title: 'Plugin',
              pluginId: 'acme.widget',
              url: 'http://127.0.0.1:9999/index.html?busToken=s3cret&sessionId=abc',
            },
          ],
        },
      ],
    } as any;
  }

  it('strips busToken from pane urls before pushing, keeping the rest of the query', async () => {
    vi.useFakeTimers();
    try {
      const agents = [pluginAgent()];
      renderSync(vi.fn(), { agents, activeAgentId: 'a1' });
      await act(async () => {
        vi.advanceTimersByTime(500);
      });

      const layoutSet = (window as any).electronAPI.layoutSet;
      expect(layoutSet).toHaveBeenCalledTimes(1);
      const sent = layoutSet.mock.calls[0][0];
      expect(JSON.stringify(sent)).not.toContain('busToken');
      expect(JSON.stringify(sent)).not.toContain('s3cret');
      const url = sent.agents[0].tabs[0].panes[0].url;
      expect(url).toContain('sessionId=abc');
      expect(sent.agents[0].tabs[0].panes[0].pluginId).toBe('acme.widget');

      // Local state keeps its token — PluginPane still has a working URL if the
      // mint is unavailable; only what leaves this client is redacted.
      expect(agents[0].tabs[0].panes[0].url).toContain('busToken=s3cret');
    } finally {
      vi.useRealTimers();
    }
  });

  it('recognises the redacted document coming back as its own echo', async () => {
    vi.useFakeTimers();
    try {
      const load = vi.fn();
      renderSync(load, { agents: [pluginAgent()], activeAgentId: 'a1' });
      await act(async () => {
        vi.advanceTimersByTime(500);
      });
      const sent = (window as any).electronAPI.layoutSet.mock.calls[0][0];
      // The hub re-broadcasts what we pushed — token-free, unlike our local
      // state. The echo-breaker projects both sides redacted, so this must be
      // recognised as ours and not re-applied as someone else's layout.
      act(() => layoutChangedCb!({ version: 1, data: sent }));
      expect(load).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
