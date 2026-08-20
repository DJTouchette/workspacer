/**
 * The "switch models and get a bunch of agents" corruption (reported from the
 * Windows machine, where every Codex model switch is secretly a respawn).
 *
 * A respawn reuses the session id. In the close→spawn→commit window, a stray
 * eviction tick used to null the card's sessionId (stopAgentForSession) and
 * the auto-adopt path then minted a SECOND card with the identical
 * deterministic id (`agent-<sessionId>`) — after which mutateAgent wrote into
 * both and React's sibling keys collided (the Overview-shows-an-agent-pane
 * symptom). These tests pin every layer of the fix: the respawn guard, the
 * eviction stand-down, adopt healing instead of appending, the sanitized
 * layout intake, and the failed-respawn un-brick.
 */
import { describe, it, expect, beforeEach, vi, type Mock } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useAgentManager, GLOBAL_WORKSPACE_ID } from '../src/hooks/useAgentManager';
import { isRespawning, resetRespawnGuard } from '../src/lib/respawnGuard';
import { wasSessionTerminated, resetTerminatedSessions } from '../src/lib/terminatedSessions';

const spawnClaude = window.electronAPI.spawnClaude as Mock;

const mkAgent = (id: string, sessionId: string | undefined, over: Record<string, unknown> = {}) =>
  ({
    id,
    name: id,
    cwd: '/x',
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
    ...over,
  }) as any;

describe('respawn duplicate-card guard', () => {
  beforeEach(() => {
    vi.useRealTimers();
    resetRespawnGuard();
    resetTerminatedSessions();
    spawnClaude.mockReset();
    spawnClaude.mockImplementation(async (opts: any) => opts.resumeSessionId ?? 'fresh-id');
  });

  it('an eviction tick mid-respawn cannot null the card (stopAgentForSession stands down)', async () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-S1', 'S1')], 'agent-S1');
    });

    // Hold the spawn open so the respawn window is wide.
    let release!: (id: string) => void;
    spawnClaude.mockImplementation(() => new Promise<string>((r) => (release = r)));
    let done: Promise<void>;
    act(() => {
      done = result.current.respawnAgentWithSettings('S1', { permissionMode: 'plan' });
    });
    await waitFor(() => expect(isRespawning('S1')).toBe(true));

    // The dying process's teardown tick arrives — it must be a no-op.
    act(() => {
      result.current.stopAgentForSession('S1');
    });
    expect(result.current.agents.find((a: any) => a.id === 'agent-S1')?.sessionId).toBe('S1');

    await act(async () => {
      release('S1');
      await done!;
    });
    const cards = result.current.agents.filter((a: any) => a.id === 'agent-S1');
    expect(cards).toHaveLength(1);
    expect(cards[0].sessionId).toBe('S1');
    expect(cards[0].permissionMode).toBe('plan');
  });

  it('adoptAgent HEALS a same-id card instead of appending a twin', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      // A mid-respawn card: deterministic id, sessionId already nulled.
      result.current.loadAgentsFromSession([mkAgent('agent-S1', undefined)], 'agent-S1');
    });
    act(() => {
      result.current.adoptAgent({ sessionId: 'S1', cwd: '/x' });
    });
    const cards = result.current.agents.filter((a: any) => a.id === 'agent-S1');
    expect(cards).toHaveLength(1);
    expect(cards[0].sessionId).toBe('S1');
    // The healed card kept its tabs — adoption didn't replace the workspace.
    expect(cards[0].tabs[0].panes[0].id).toBe('agent-S1-p');
  });

  it('a failed respawn leaves an honest Stopped card, not a tombstoned zombie', async () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-S1', 'S1')], 'agent-S1');
    });
    spawnClaude.mockRejectedValue(new Error('daemon offline'));
    await act(async () => {
      await result.current.respawnAgentWithSettings('S1', { model: 'opus' });
    });
    const a = result.current.agents.find((x: any) => x.id === 'agent-S1');
    expect(a.sessionId).toBeUndefined();
    expect(a.lastSessionId).toBe('S1');
    // The tombstone is lifted so the card can be respawned by hand later.
    expect(wasSessionTerminated('S1')).toBe(false);
  });

  it('a nested worker (parentId) is DROPPED on session end, not left as a Stopped tombstone', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession(
        [mkAgent('mgr', 'MGR'), mkAgent('worker', 'W1', { parentId: 'mgr' })],
        'mgr',
      );
    });
    act(() => {
      result.current.stopAgentForSession('W1');
    });
    // The ephemeral sub-agent card is gone entirely — the manager relayed its
    // result via the [fleet] wake, so a lingering Stopped sub-card is clutter.
    expect(result.current.agents.find((a: any) => a.id === 'worker')).toBeUndefined();
    // …while a top-level card in the same tick still tombstones (resumable).
    act(() => {
      result.current.stopAgentForSession('MGR');
    });
    const mgr = result.current.agents.find((a: any) => a.id === 'mgr');
    expect(mgr).toBeDefined();
    expect(mgr.sessionId).toBeUndefined();
    expect(mgr.lastSessionId).toBe('MGR');
  });
});

describe('openManagedTerminal (facade open_terminal)', () => {
  beforeEach(() => {
    resetRespawnGuard();
    resetTerminatedSessions();
  });

  it('opens a visible terminal tab NESTED under the calling agent, running the command', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('mgr', 'MGR')], 'mgr');
    });
    act(() => {
      result.current.openManagedTerminal({
        parentSessionId: 'MGR',
        cwd: '/home/u/Work/preheat',
        command: 'npm run dev',
        label: 'preheat dev server',
      });
    });
    const mgr = result.current.agents.find((a: any) => a.id === 'mgr');
    // A new tab was added to the manager's own workspace — not a new card.
    expect(result.current.agents.filter((a: any) => !a.global)).toHaveLength(1);
    const termTab = mgr.tabs.find((t: any) => t.panes[0].type === 'terminal');
    expect(termTab).toBeDefined();
    expect(termTab.panes[0].cwd).toBe('/home/u/Work/preheat');
    expect(termTab.panes[0].initialCommand).toBe('npm run dev');
    expect(termTab.panes[0].title).toBe('preheat dev server');
    expect(mgr.activeTabId).toBe(termTab.id);
  });

  it('falls back to a standalone terminal card when no agent owns the parent session', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('mgr', 'MGR')], 'mgr');
    });
    act(() => {
      result.current.openManagedTerminal({
        parentSessionId: 'GHOST',
        cwd: '/tmp/x',
        command: 'python -m http.server',
        label: 'static server',
      });
    });
    // A standalone card exists — the process is never invisible even with no parent.
    expect(result.current.agents.filter((a: any) => !a.global)).toHaveLength(2);
    const card = result.current.agents.find((a: any) => a.name === 'static server');
    expect(card).toBeDefined();
    expect(card.tabs[0].panes[0].type).toBe('terminal');
    expect(card.tabs[0].panes[0].initialCommand).toBe('python -m http.server');
  });
});

describe('loadAgentsFromSession sanitization', () => {
  beforeEach(() => {
    resetRespawnGuard();
    resetTerminatedSessions();
  });

  it('collapses duplicate card ids, keeping the live copy', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession(
        [mkAgent('agent-S1', undefined), mkAgent('agent-S1', 'S1')],
        'agent-S1',
      );
    });
    const cards = result.current.agents.filter((a: any) => a.id === 'agent-S1');
    expect(cards).toHaveLength(1);
    expect(cards[0].sessionId).toBe('S1');
  });

  it("evicts a non-global card squatting on the 'global' id", () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession(
        [mkAgent(GLOBAL_WORKSPACE_ID, 'S9'), mkAgent('agent-S1', 'S1')],
        'agent-S1',
      );
    });
    const globals = result.current.agents.filter((a: any) => a.id === GLOBAL_WORKSPACE_ID);
    expect(globals).toHaveLength(1);
    expect(globals[0].global).toBe(true);
    expect(globals[0].sessionId).toBeUndefined();
    // The squatter survives under its deterministic id, not as the Overview.
    expect(
      result.current.agents.some((a: any) => a.id === 'agent-S9' && a.sessionId === 'S9'),
    ).toBe(true);
  });

  it('re-homes a global-flagged card that lost the fixed id', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession(
        [mkAgent('weird-global-id', undefined, { global: true }), mkAgent('agent-S1', 'S1')],
        'agent-S1',
      );
    });
    const globals = result.current.agents.filter((a: any) => a.global);
    expect(globals).toHaveLength(1);
    expect(globals[0].id).toBe(GLOBAL_WORKSPACE_ID);
  });

  it('an Overview whose only tab dies in normalization still gets its pane back', () => {
    const { result } = renderHook(() => useAgentManager());
    const global = {
      id: GLOBAL_WORKSPACE_ID,
      name: 'Overview',
      cwd: '',
      global: true,
      tabs: [
        {
          id: 'g-t',
          title: 'Notes',
          panes: [{ id: 'g-p', type: 'notes', title: 'Notes' }], // retired type → filtered
          activePaneId: 'g-p',
        },
      ],
      activeTabId: 'g-t',
    } as any;
    act(() => {
      result.current.loadAgentsFromSession([global, mkAgent('agent-S1', 'S1')], 'agent-S1');
    });
    const g = result.current.agents.find((a: any) => a.id === GLOBAL_WORKSPACE_ID);
    expect(g.tabs.length).toBeGreaterThan(0);
    expect(g.tabs[0].panes[0].type).toBe('overview');
    expect(g.tabs.some((t: any) => t.id === g.activeTabId)).toBe(true);
  });
});
