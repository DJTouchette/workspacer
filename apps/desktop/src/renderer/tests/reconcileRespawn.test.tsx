/**
 * Boot reconciliation + auto-respawn (the machine-reboot recovery path).
 *
 * After a reboot every restored agent card carries a sessionId that claudemon
 * only holds as a stopped-but-resumable row. reconcileAgents must mark those
 * agents stopped and — on the boot path (`respawnStopped`) — immediately
 * respawn them resuming the old id, re-pointing their panes so the pane shows
 * the fetching state instead of spinning on "Connecting…" forever.
 */
import { describe, it, expect, beforeEach, vi, type Mock } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';

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

describe('useAgentManager.reconcileAgents — boot auto-respawn', () => {
  beforeEach(() => {
    spawnClaude.mockClear();
    // Resume spawns return the same pinned id (claude's canonical-id contract).
    spawnClaude.mockImplementation(async (opts: any) => opts.resumeSessionId ?? 'fresh-id');
  });

  it('marks dead agents stopped, then respawns them resuming the old session id', async () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-a', 'S1')], 'agent-a');
    });
    act(() => {
      result.current.reconcileAgents(new Set(), { respawnStopped: true });
    });
    expect(spawnClaude).toHaveBeenCalledTimes(1);
    expect(spawnClaude).toHaveBeenCalledWith(expect.objectContaining({ resumeSessionId: 'S1' }));
    await waitFor(() => {
      const a = result.current.agents.find((x: any) => x.id === 'agent-a');
      expect(a.sessionId).toBe('S1');
      expect(a.lastSessionId).toBeUndefined();
    });
    // The pane is re-pointed with the restore marker so it shows the
    // "Fetching session…" state while the transcript replays.
    const a = result.current.agents.find((x: any) => x.id === 'agent-a');
    const pane = a.tabs[0].panes[0];
    expect(pane.attachSessionId).toBe('S1');
    expect(pane.expectHistory).toBe(true);
  });

  it('leaves agents with live sessions untouched', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession(
        [mkAgent('agent-a', 'S1'), mkAgent('agent-b', 'S2')],
        'agent-a',
      );
    });
    act(() => {
      result.current.reconcileAgents(new Set(['S1']), { respawnStopped: true });
    });
    const a = result.current.agents.find((x: any) => x.id === 'agent-a');
    expect(a.sessionId).toBe('S1');
    expect(spawnClaude).toHaveBeenCalledTimes(1);
    expect(spawnClaude).toHaveBeenCalledWith(expect.objectContaining({ resumeSessionId: 'S2' }));
  });

  it('does not respawn without the respawnStopped flag (plain reconcile)', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-a', 'S1')], 'agent-a');
    });
    act(() => {
      result.current.reconcileAgents(new Set());
    });
    const a = result.current.agents.find((x: any) => x.id === 'agent-a');
    expect(a.sessionId).toBeUndefined();
    expect(a.lastSessionId).toBe('S1');
    expect(spawnClaude).not.toHaveBeenCalled();
  });

  it('ignores agents that were already stopped before the layout was saved', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession(
        [mkAgent('agent-a', undefined, { lastSessionId: 'OLD' })],
        'agent-a',
      );
    });
    act(() => {
      result.current.reconcileAgents(new Set(), { respawnStopped: true });
    });
    expect(spawnClaude).not.toHaveBeenCalled();
    const a = result.current.agents.find((x: any) => x.id === 'agent-a');
    expect(a.lastSessionId).toBe('OLD');
  });

  it('carries the saved launch settings into the respawn', () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession(
        [
          mkAgent('agent-a', 'S1', {
            model: 'claude-opus-4-8',
            permissionMode: 'acceptEdits',
            transport: 'stream',
          }),
        ],
        'agent-a',
      );
    });
    act(() => {
      result.current.reconcileAgents(new Set(), { respawnStopped: true });
    });
    expect(spawnClaude).toHaveBeenCalledWith(
      expect.objectContaining({
        resumeSessionId: 'S1',
        model: 'claude-opus-4-8',
        permissionMode: 'acceptEdits',
        transport: 'stream',
      }),
    );
  });
});
