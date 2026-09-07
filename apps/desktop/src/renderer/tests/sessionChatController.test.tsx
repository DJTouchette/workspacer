import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  usePublishSessionChatController,
  useSessionChatController,
  waitForSessionChatController,
} from '../src/hooks/useSessionChatController';
const controller = (text: string) => ({
  send: vi.fn(async () => ({ ok: true })),
  conversation: [{ role: 'user' as const, content: text, timestamp: 1 }],
  pending: [],
});
afterEach(() => vi.useRealTimers());
describe('shared chat controller ownership', () => {
  it('projects the exact owner queue and excludes another pane or session', () => {
    const owner = controller('owner');
    const watch = controller('watch');
    const other = controller('other');
    const host = renderHook(() => {
      usePublishSessionChatController('s1', 'p1', owner);
      usePublishSessionChatController('s1', 'watch', watch);
      usePublishSessionChatController('s2', 'p1', other);
    });
    const card = renderHook(() => useSessionChatController('s1', 'p1'));
    expect(card.result.current).toBe(owner);
    expect(card.result.current?.conversation).toBe(owner.conversation);
    expect(card.result.current?.pending).toBe(owner.pending);
    host.unmount();
    expect(card.result.current).toBeUndefined();
  });
  it('waits for the restored owning pane without borrowing a watch', async () => {
    const watch = renderHook(() =>
      usePublishSessionChatController('restore', 'watch', controller('watch')),
    );
    const owner = controller('restored owner');
    let paneId: string | undefined;
    const pending = waitForSessionChatController('restore', () => paneId);
    paneId = 'restored';
    const host = renderHook(() => usePublishSessionChatController('restore', paneId!, owner));
    await expect(pending).resolves.toBe(owner);
    host.unmount();
    watch.unmount();
  });
  it('times out without sending and removes the previous session when an owner changes', async () => {
    vi.useFakeTimers();
    const value = controller('session');
    const host = renderHook(({ id }) => usePublishSessionChatController(id, 'pane', value), {
      initialProps: { id: 'old' },
    });
    const card = renderHook(() => useSessionChatController('old', 'pane'));
    host.rerender({ id: 'new' });
    expect(card.result.current).toBeUndefined();
    const pending = waitForSessionChatController('old', () => 'pane', 10);
    await act(async () => vi.advanceTimersByTime(10));
    await expect(pending).resolves.toBeUndefined();
    expect(value.send).not.toHaveBeenCalled();
  });
});
