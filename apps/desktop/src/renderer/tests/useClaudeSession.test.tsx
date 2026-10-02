/**
 * Regression test: useClaudeSession must not keep showing a previous session's
 * snapshot after the tracked id changes (e.g. a pane re-pointed via respawn) or
 * clears (pane detached). Leaving stale state surfaces another session's status
 * and pending prompts in the wrong pane.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
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
    onClaudeSessionDetail: undefined,
    getClaudeSession: vi.fn((id: string) =>
      Promise.resolve(id === 'A' ? { sessionId: 'A', status: 'running' } : null),
    ),
  };
});
afterEach(() => vi.useRealTimers());

describe('useClaudeSession — stale state on id change', () => {
  it('subscribes to detail only while active and preserves unchanged turn identity', async () => {
    let detail: (snapshot: any) => void = () => {};
    const off = vi.fn();
    window.electronAPI.onClaudeSessionDetail = vi.fn((_id, cb) => {
      detail = cb;
      return off;
    });
    window.electronAPI.getClaudeSession = vi.fn().mockResolvedValue(fullSnapshot());
    const { result, rerender } = renderHook(
      ({ active }) => useClaudeSession({ ptySessionId: 'A', active }),
      { initialProps: { active: true } },
    );
    await waitFor(() => expect(result.current.session?.conversation).toHaveLength(20));
    const first = result.current.session!.conversation[0];
    const next = fullSnapshot();
    next.conversation[19].content = 'new streamed reply';
    act(() => detail(next));
    expect(result.current.session!.conversation[0]).toBe(first);
    expect(result.current.session!.conversation[19].content).toBe('new streamed reply');
    expect(window.electronAPI.onClaudeSessionUpdate).not.toHaveBeenCalled();
    rerender({ active: false });
    expect(off).toHaveBeenCalledOnce();
    await waitFor(() => expect(result.current.session?.conversation).toHaveLength(12));
    expect(window.electronAPI.getClaudeSession).toHaveBeenLastCalledWith('A', true);
  });

  it('does not let an older full fetch overwrite a newer detail update', async () => {
    let resolve: (snapshot: any) => void = () => {};
    let detail: (snapshot: any) => void = () => {};
    window.electronAPI.getClaudeSession = vi.fn(
      () =>
        new Promise((r) => {
          resolve = r;
        }),
    );
    window.electronAPI.onClaudeSessionDetail = vi.fn((_id, cb) => {
      detail = cb;
      return () => {};
    });
    const { result } = renderHook(() => useClaudeSession({ ptySessionId: 'A' }));
    const current = fullSnapshot(25);
    act(() => detail(current));
    await act(async () => resolve(fullSnapshot(20)));
    expect(result.current.session?.conversation).toHaveLength(25);
  });

  it('recovers a full history prefix when a newer bus window arrives during activation', async () => {
    let resolve: (snapshot: any) => void = () => {};
    let update: (id: string, snapshot: any) => void = () => {};
    window.electronAPI.getClaudeSession = vi.fn(
      () =>
        new Promise((r) => {
          resolve = r;
        }),
    );
    window.electronAPI.onClaudeSessionUpdate = vi.fn((cb) => {
      update = cb;
      return () => {};
    });
    const { result } = renderHook(() => useClaudeSession({ ptySessionId: 'A' }));
    const full = fullSnapshot();
    const newerWindow = {
      ...full,
      ambientState: 'streaming',
      conversationOffset: 8,
      conversation: full.conversation.slice(8).map((t) => ({ ...t })),
    };
    newerWindow.conversation[11].content = 'latest text';
    act(() => update('A', newerWindow));
    await act(async () => resolve(full));
    expect(result.current.session?.conversation).toHaveLength(20);
    expect(result.current.session?.conversation[19].content).toBe('latest text');
    expect(result.current.session?.ambientState).toBe('streaming');
  });

  it('never replays a hidden pending running snapshot after the session ends', async () => {
    vi.useFakeTimers();
    let update: (id: string, snapshot: any) => void = () => {};
    window.electronAPI.onClaudeSessionUpdate = vi.fn((cb) => {
      update = cb;
      return () => {};
    });
    const { result } = renderHook(() => useClaudeSession({ ptySessionId: 'A', active: false }));
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      update('A', { ...fullSnapshot(), ambientState: 'streaming' });
      update('A', { ...fullSnapshot(), status: 'ended' });
    });
    expect(result.current.session?.status).toBe('ended');
    act(() => vi.advanceTimersByTime(1500));
    expect(result.current.session?.status).toBe('ended');
  });

  it('discards an older queued hidden update when an explicit refresh succeeds', async () => {
    vi.useFakeTimers();
    let update: (id: string, snapshot: any) => void = () => {};
    window.electronAPI.onClaudeSessionUpdate = vi.fn((cb) => {
      update = cb;
      return () => {};
    });
    window.electronAPI.getClaudeSession = vi.fn().mockResolvedValue(fullSnapshot());
    const { result } = renderHook(() => useClaudeSession({ ptySessionId: 'A', active: false }));
    await act(async () => {
      await Promise.resolve();
    });
    act(() => update('A', { ...fullSnapshot(), ambientState: 'streaming' }));
    await act(async () => result.current.refresh());
    expect(result.current.session?.ambientState).toBe('idle');
    act(() => vi.advanceTimersByTime(1500));
    expect(result.current.session?.ambientState).toBe('idle');
  });
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
