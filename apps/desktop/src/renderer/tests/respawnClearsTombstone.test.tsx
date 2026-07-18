/**
 * Regression: restart-with-settings tombstones the old session id, then resumes
 * onto the SAME id (claudeSpawn/managedSpawn both do
 * `opts.resumeSessionId || randomUUID()`). If the tombstone is never lifted,
 * App.tsx drops every subsequent live snapshot for the restarted (actually
 * running) session and marks the agent Stopped forever. respawnAgentWithSettings
 * must clear the tombstone once the resumed session is live again.
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';
import { wasSessionTerminated, resetTerminatedSessions } from '../src/lib/terminatedSessions';

const mkAgent = (id: string, sessionId: string) =>
  ({
    id,
    name: id,
    cwd: '/x',
    sessionId,
    model: 'sonnet',
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

describe('respawnAgentWithSettings — resume reuses the tombstoned id', () => {
  beforeEach(() => {
    resetTerminatedSessions();
    // The resume path pins the same id: spawn returns resumeSessionId unchanged.
    (window.electronAPI.spawnClaude as any) = vi
      .fn()
      .mockImplementation((opts: any) => Promise.resolve(opts.resumeSessionId ?? 'fresh'));
    (window.electronAPI.claudeClose as any) = vi.fn().mockResolvedValue(undefined);
  });

  it('lifts the tombstone for the resumed (reused) session id', async () => {
    const { result } = renderHook(() => useAgentManager());
    act(() => {
      result.current.loadAgentsFromSession([mkAgent('agent-a', 'S1')], 'agent-a');
    });

    await act(async () => {
      await result.current.respawnAgentWithSettings('S1', { model: 'opus' });
    });

    // The restarted session reuses id 'S1'; it must NOT stay tombstoned, or its
    // live snapshots get dropped and it renders Stopped forever.
    expect(wasSessionTerminated('S1')).toBe(false);
    expect(result.current.agents.find((a: any) => a.id === 'agent-a')?.sessionId).toBe('S1');
  });
});
