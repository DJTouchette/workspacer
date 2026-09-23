import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useSessionSnapshots, FLEET_SNAPSHOT_FLUSH_MS } from '../src/hooks/useSessionSnapshots';
import type { ClaudeSessionSnapshot } from '../src/types/claudeSession';

let push: (id: string, snapshot: any) => void;
const snapshot = (id = 'a', content = '') =>
  ({
    sessionId: id,
    status: 'running',
    ambientState: 'streaming',
    conversation: [{ role: 'assistant', content, timestamp: 1 }],
  }) as ClaudeSessionSnapshot;
beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(window.electronAPI.getAllClaudeSessions).mockResolvedValue([snapshot()]);
  vi.mocked(window.electronAPI.onClaudeSessionUpdate).mockImplementation((cb) => {
    push = cb;
    return vi.fn();
  });
});
afterEach(() => vi.useRealTimers());

it('publishes a fleet streaming burst once and preserves unchanged statuses', async () => {
  const stop = vi.fn();
  const { result } = renderHook(() => useSessionSnapshots(stop));
  await act(async () => {});
  const before = result.current.snapshotBySession;
  const statuses = result.current.statusBySession;
  for (let i = 1; i <= 30; i++) act(() => push('a', snapshot('a', String(i))));
  expect(result.current.snapshotBySession).toBe(before);
  act(() => vi.advanceTimersByTime(FLEET_SNAPSHOT_FLUSH_MS));
  expect(result.current.snapshotBySession.a.conversation[0].content).toBe('30');
  expect(result.current.statusBySession).toBe(statuses);
});

it('publishes changed decisions immediately and never resurrects a queued ended session', async () => {
  const stop = vi.fn();
  const { result } = renderHook(() => useSessionSnapshots(stop));
  await act(async () => {});
  act(() => push('a', snapshot('a', 'pending')));
  const approval = { toolName: 'Bash', toolInput: { command: 'first' }, timestamp: 2 };
  act(() =>
    push('a', { ...snapshot(), ambientState: 'waiting_approval', pendingApproval: approval }),
  );
  expect(result.current.snapshotBySession.a.pendingApproval).toEqual(approval);
  act(() =>
    push('a', {
      ...snapshot(),
      ambientState: 'waiting_approval',
      pendingApproval: { ...approval, toolInput: { command: 'changed' } },
    }),
  );
  expect(result.current.snapshotBySession.a.pendingApproval?.toolInput.command).toBe('changed');
  act(() =>
    push('a', {
      ...snapshot(),
      ambientState: 'waiting_approval',
      pendingApproval: { ...approval, toolInput: { command: 'changed' } },
    }),
  );
  act(() => push('a', { ...snapshot(), status: 'ended' }));
  act(() => vi.runAllTimers());
  expect(result.current.snapshotBySession.a).toBeUndefined();
  expect(stop).toHaveBeenCalledWith('a');
});

it('discards queued updates on explicit prune and cancels its timer on unmount', async () => {
  const stop = vi.fn();
  const { result, unmount } = renderHook(() => useSessionSnapshots(stop));
  await act(async () => {});
  act(() => push('a', snapshot('a', 'queued')));
  act(() => result.current.pruneSession('a'));
  act(() => vi.runAllTimers());
  expect(result.current.snapshotBySession.a).toBeUndefined();
  act(() => push('b', snapshot('b')));
  act(() => push('b', snapshot('b', 'queued')));
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});

it('does not resurrect a session from a list fetched before its end', async () => {
  let resolve!: (snapshots: any[]) => void;
  vi.mocked(window.electronAPI.getAllClaudeSessions).mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  const stop = vi.fn();
  const { result } = renderHook(() => useSessionSnapshots(stop));
  act(() => push('a', snapshot()));
  act(() => push('a', { ...snapshot(), status: 'ended' }));
  await act(async () => resolve([snapshot()]));
  expect(result.current.snapshotBySession.a).toBeUndefined();
});

it('does not let a pre-refresh queued observation overwrite the newer fetched state', async () => {
  const stop = vi.fn();
  const { result } = renderHook(() => useSessionSnapshots(stop));
  await act(async () => {});
  act(() => push('a', snapshot('a', 'older queued')));
  vi.mocked(window.electronAPI.getAllClaudeSessions).mockResolvedValue([
    snapshot('a', 'newer fetched'),
  ]);
  await act(async () => result.current.refreshSessionSnapshots());
  expect(result.current.snapshotBySession.a.conversation[0].content).toBe('newer fetched');
  act(() => vi.runAllTimers());
  expect(result.current.snapshotBySession.a.conversation[0].content).toBe('newer fetched');
});

it('forgets decisions for vanished sessions so a returning session publishes immediately', async () => {
  const stop = vi.fn();
  const { result } = renderHook(() => useSessionSnapshots(stop));
  await act(async () => {});
  vi.mocked(window.electronAPI.getAllClaudeSessions).mockResolvedValue([]);
  await act(async () => result.current.refreshSessionSnapshots());
  expect(result.current.snapshotBySession.a).toBeUndefined();
  act(() => push('a', snapshot('a', 'returned')));
  expect(result.current.snapshotBySession.a?.conversation[0].content).toBe('returned');
});
