import { describe, it, expect, vi, beforeEach } from 'vitest';

const callHub = vi.fn();
let eventListener: ((ev: { type: string; hub?: string; data?: unknown }) => void) | null = null;
let connectListener: (() => void) | null = null;
vi.mock('./hubClient', () => ({
  callHub: (...a: unknown[]) => callHub(...a),
  subscribeHubEvents: (l: typeof eventListener) => {
    eventListener = l;
    return () => {};
  },
  subscribeHubConnected: (l: typeof connectListener) => {
    connectListener = l;
    return () => {};
  },
}));

import { openSessionIds, resetSessionOpenCacheForTests } from './sessionOpenCache';

const ids = async () => [...(await openSessionIds())].sort();

describe('sessionOpenCache', () => {
  beforeEach(() => {
    resetSessionOpenCacheForTests();
    callHub.mockReset();
    eventListener = null;
    connectListener = null;
  });

  it('reads sessionOpen.get once, then follows changed events by version', async () => {
    callHub.mockResolvedValueOnce({ version: 2, open: { a: 1, b: 2 } });
    expect(await ids()).toEqual(['a', 'b']);
    expect(callHub).toHaveBeenCalledWith('sessionOpen.get', {});

    eventListener!({ type: 'sessionOpen.changed', data: { version: 3, open: { a: 1 } } });
    expect(await ids()).toEqual(['a']);
    // Stale and federated events are ignored.
    eventListener!({ type: 'sessionOpen.changed', data: { version: 1, open: { z: 1 } } });
    eventListener!({
      type: 'sessionOpen.changed',
      hub: 'peer',
      data: { version: 9, open: { p: 1 } },
    });
    expect(await ids()).toEqual(['a']);
    expect(callHub).toHaveBeenCalledTimes(1);
  });

  it('re-reads after a reconnect, even when the hub reset its version', async () => {
    callHub.mockResolvedValueOnce({ version: 5, open: { a: 1 } });
    expect(await ids()).toEqual(['a']);
    callHub.mockResolvedValueOnce({ version: 1, open: { b: 1 } });
    connectListener!();
    await vi.waitFor(async () => expect(await ids()).toEqual(['b']));
  });

  it('is an empty set when the hub is unreachable or too old', async () => {
    callHub.mockRejectedValue(new Error('unknown method sessionOpen.get'));
    expect(await ids()).toEqual([]);
    callHub.mockResolvedValueOnce({ version: 1, open: { late: 1 } });
    expect(await ids()).toEqual(['late']);
  });
});
