/**
 * Regression test: useClaudeSession must not keep showing a previous session's
 * snapshot after the tracked id changes (e.g. a pane re-pointed via respawn) or
 * clears (pane detached). Leaving stale state surfaces another session's status
 * and pending prompts in the wrong pane.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useClaudeSession } from '../src/hooks/useClaudeSession';

function fullSnapshot(turns = 20) {
  return {
    sessionId: 'A',
    cwd: '/work',
    ptyId: 'A',
    status: 'active',
    conversation: Array.from({ length: turns }, (_, i) => ({
      role: i % 2 === 0 ? 'user' : 'assistant',
      content: i === turns - 1 ? 'x'.repeat(6000) : `turn ${i}`,
      timestamp: i,
    })),
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    pendingQuestions: null,
    subagents: [],
    workflows: [],
    ambientState: 'idle',
    lastActivity: 1,
    totalToolCalls: 0,
    usage: null,
  };
}

beforeEach(() => {
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    onClaudeSessionUpdate: vi.fn().mockReturnValue(() => {}),
    getClaudeSession: vi.fn((id: string) =>
      Promise.resolve(id === 'A' ? { sessionId: 'A', status: 'running' } : null),
    ),
  };
});

describe('useClaudeSession — stale state on id change', () => {
  it('clears the previous session when re-pointed to an id with no snapshot', async () => {
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => useClaudeSession({ ptySessionId: id, active: true }),
      { initialProps: { id: 'A' as string | null } },
    );

    await waitFor(() => expect(result.current.session?.sessionId).toBe('A'));

    rerender({ id: 'B' }); // B has no snapshot yet
    await act(async () => {
      await Promise.resolve();
    });

    expect(result.current.session).toBeNull();
  });

  it('clears the session when the pane is detached (id becomes null)', async () => {
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => useClaudeSession({ ptySessionId: id, active: true }),
      { initialProps: { id: 'A' as string | null } },
    );

    await waitFor(() => expect(result.current.session?.sessionId).toBe('A'));

    rerender({ id: null });
    await act(async () => {
      await Promise.resolve();
    });

    expect(result.current.session).toBeNull();
  });

  it('keeps inactive panes compact and refetches the full snapshot when activated', async () => {
    (window as any).electronAPI.getClaudeSession = vi.fn(() => Promise.resolve(fullSnapshot()));

    const { result, rerender } = renderHook(
      ({ active }: { active: boolean }) => useClaudeSession({ ptySessionId: 'A', active }),
      { initialProps: { active: false } },
    );

    await waitFor(() => expect(result.current.session?.sessionId).toBe('A'));
    expect(result.current.session?.conversation).toHaveLength(12);
    expect(result.current.session?.conversation.at(-1)?.content).toContain('[truncated ');

    rerender({ active: true });

    await waitFor(() => expect(result.current.session?.conversation).toHaveLength(20));
  });
});
