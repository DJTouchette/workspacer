/**
 * Regression test: useClaudeSession must not keep showing a previous session's
 * snapshot after the tracked id changes (e.g. a pane re-pointed via respawn) or
 * clears (pane detached). Leaving stale state surfaces another session's status
 * and pending prompts in the wrong pane.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useClaudeSession } from '../src/hooks/useClaudeSession';

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
    await act(async () => { await Promise.resolve(); });

    expect(result.current.session).toBeNull();
  });

  it('clears the session when the pane is detached (id becomes null)', async () => {
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => useClaudeSession({ ptySessionId: id, active: true }),
      { initialProps: { id: 'A' as string | null } },
    );

    await waitFor(() => expect(result.current.session?.sessionId).toBe('A'));

    rerender({ id: null });
    await act(async () => { await Promise.resolve(); });

    expect(result.current.session).toBeNull();
  });
});
