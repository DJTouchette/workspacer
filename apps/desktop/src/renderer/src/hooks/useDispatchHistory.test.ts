import { afterEach, describe, expect, it, vi } from 'vitest';
import { createDispatchHistoryPoller } from './useDispatchHistory';
import type { DispatchHistoryResponse, DispatchTask } from '../../../main/shared/dispatchHistory';
const response: DispatchHistoryResponse = { available: true, tasks: [] };
afterEach(() => vi.useRealTimers());
describe('shared task history polling', () => {
  it('shares an immediate read and each poll, then releases the last subscription', async () => {
    vi.useFakeTimers();
    const read = vi.fn(async () => response);
    const poller = createDispatchHistoryPoller(read);
    const first = poller.subscribe(vi.fn());
    const second = poller.subscribe(vi.fn());
    await poller.refresh();
    expect(read).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(3000);
    expect(read).toHaveBeenCalledTimes(2);
    first();
    await vi.advanceTimersByTimeAsync(3000);
    expect(read).toHaveBeenCalledTimes(3);
    second();
    await vi.advanceTimersByTimeAsync(6000);
    expect(read).toHaveBeenCalledTimes(3);
    expect(vi.getTimerCount()).toBe(0);
    expect(poller.getSnapshot().data).toBeUndefined();
  });
  it('does not overlap slow reads and suppresses an old response after an edit', async () => {
    vi.useFakeTimers();
    const task = { taskId: 'one', title: 'before' } as DispatchTask;
    let finish!: (data: DispatchHistoryResponse) => void;
    const read = vi
      .fn()
      .mockResolvedValueOnce({ available: true, tasks: [task] })
      .mockImplementation(
        () =>
          new Promise<DispatchHistoryResponse>((resolve) => {
            finish = resolve;
          }),
      );
    const poller = createDispatchHistoryPoller(read);
    const stop = poller.subscribe(vi.fn());
    await poller.refresh();
    const old = poller.refresh();
    await vi.advanceTimersByTimeAsync(9000);
    expect(read).toHaveBeenCalledTimes(2);
    poller.updateTask({ ...task, title: 'edited' });
    finish({ available: true, tasks: [task] });
    await old;
    expect(poller.getSnapshot().data).toMatchObject({ tasks: [{ title: 'edited' }] });
    stop();
  });
  it('ignores requests from an unmounted generation and recovers from errors', async () => {
    let finish!: (data: DispatchHistoryResponse) => void;
    const read = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise<DispatchHistoryResponse>((resolve) => {
            finish = resolve;
          }),
      )
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValue(response);
    const poller = createDispatchHistoryPoller(read);
    const stop = poller.subscribe(vi.fn());
    const old = poller.refresh();
    await Promise.resolve();
    stop();
    const stopNext = poller.subscribe(vi.fn());
    await poller.refresh();
    expect(poller.getSnapshot().error).toContain('offline');
    finish({ available: false, reason: 'old host' });
    await old;
    expect(poller.getSnapshot().error).toContain('offline');
    await poller.refresh();
    expect(poller.getSnapshot()).toEqual({ data: response, error: '', busy: false });
    stopNext();
  });
});
