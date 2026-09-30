import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HubBusClient } from './hubBusClient';
import { createWebBackend } from './webBackend';

/**
 * Drive-able WebSocket stand-in: records every instance the client opens and
 * exposes open()/die() so a test can simulate the connect / drop / reconnect
 * lifecycle without a real socket. Mirrors the bits of the DOM WebSocket the
 * client touches (readyState, the four on* handlers, send/close, static OPEN).
 */
class FakeWS {
  static instances: FakeWS[] = [];
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  readyState = FakeWS.CONNECTING;
  onopen: ((e?: unknown) => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onerror: ((e?: unknown) => void) | null = null;
  onclose: ((e: { code: number }) => void) | null = null;
  sent: string[] = [];

  constructor(public url: string) {
    FakeWS.instances.push(this);
  }
  send(data: string): void {
    this.sent.push(data);
  }
  close(): void {
    if (this.readyState === FakeWS.CLOSED) return;
    this.readyState = FakeWS.CLOSED;
    this.onclose?.({ code: 1000 });
  }
  /** Simulate the server accepting the connection. */
  open(): void {
    this.readyState = FakeWS.OPEN;
    this.onopen?.();
  }
  /** Simulate the network dropping the socket with a retryable close. */
  die(code = 1006): void {
    this.readyState = FakeWS.CLOSED;
    this.onclose?.({ code });
  }
}

describe('HubBusClient reconnect handling', () => {
  beforeEach(() => {
    FakeWS.instances = [];
    vi.stubGlobal('WebSocket', FakeWS as unknown as typeof WebSocket);
    const storage = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value);
      },
      removeItem: (key: string) => {
        storage.delete(key);
      },
    });
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('stays disconnected after a machine stop across timers and restart, then wakes explicitly', async () => {
    const client = new HubBusClient('tok', 'wss://power-test.example/bus');
    client.start();
    const ws = FakeWS.instances[0];
    ws.open();
    const pending = client
      .call('agents.sendMessage', { message: 'queued' })
      .catch((e: Error) => e.message);
    ws.die(4001);
    expect(await pending).toContain('stop request');
    expect(client.isPowerPaused()).toBe(true);
    vi.advanceTimersByTime(120000);
    document.dispatchEvent(new Event('visibilitychange'));
    window.dispatchEvent(new Event('online'));
    expect(FakeWS.instances).toHaveLength(1);
    const reloaded = new HubBusClient('tok', 'wss://power-test.example/bus');
    reloaded.start();
    expect(FakeWS.instances).toHaveLength(1);
    await expect(reloaded.call('agents.sendMessage')).rejects.toThrow('stop request');
    reloaded.resumeMachine();
    expect(FakeWS.instances).toHaveLength(2);
    FakeWS.instances[1].open();
    expect(FakeWS.instances[1].sent.some((frame) => frame.includes('queued'))).toBe(false);
    reloaded.stop();
    client.stop();
  });

  it('web hubPublish serializes the publish wire operation rather than an unprovided RPC', async () => {
    const backend = createWebBackend('tok', 'ws://publication.example/bus');
    const ws = FakeWS.instances[0];
    ws.open();
    const event = {
      type: 'ui.pane.opened',
      source: 'workspacer.ui',
      data: { title: 'Exact 🦀', nested: [1, false] },
    };
    const pending = backend.hubPublish!(event);
    const wire = JSON.parse(ws.sent[ws.sent.length - 1]);
    // The old implementation receives this real protocol refusal shape and
    // silently resolves. The assertion below independently inspects its wire.
    if (wire.op === 'call')
      ws.onmessage?.({
        data: JSON.stringify({ op: 'error', id: wire.id, error: 'no provider for __publish' }),
      });
    await pending;
    expect(wire).toEqual({ op: 'publish', event });
  });

  it('best-effort publication drops disconnected, failed-send and stopped events without replay', () => {
    const client = new HubBusClient('tok', 'ws://publication-lifecycle.example/bus');
    client.start();
    const first = FakeWS.instances[0];
    const event = { type: 'ui.pane.opened', source: 'workspacer.ui', data: { exact: true } };
    client.publish(event); // CONNECTING is not a durable outbox.
    first.open();
    expect(first.sent.filter((frame) => JSON.parse(frame).op === 'publish')).toEqual([]);
    client.publish(event);
    expect(JSON.parse(first.sent[first.sent.length - 1])).toEqual({ op: 'publish', event });
    first.die();
    client.publish({ ...event, data: { mustNotReplay: true } });
    vi.advanceTimersByTime(1000);
    const second = FakeWS.instances[1];
    second.open();
    expect(second.sent.filter((frame) => JSON.parse(frame).op === 'publish')).toEqual([]);
    const send = vi.spyOn(second, 'send').mockImplementation(() => {
      throw new Error('closed during send');
    });
    expect(() => client.publish(event)).not.toThrow();
    send.mockRestore();
    client.stop();
    client.publish(event);
    expect(second.sent.filter((frame) => JSON.parse(frame).op === 'publish')).toEqual([]);
  });

  it('uncorrelated server publication refusal cannot settle an RPC or create a retry', async () => {
    const client = new HubBusClient('view-token', 'ws://publication-refusal.example/bus');
    client.start();
    const ws = FakeWS.instances[0];
    ws.open();
    ws.onmessage?.({
      data: JSON.stringify({ op: 'hello', scope: 'view', methods: ['config.get'] }),
    });
    const pending = client.call('config.get');
    const call = JSON.parse(ws.sent[ws.sent.length - 1]);
    client.publish({ type: 'ui.pane.opened', data: { denied: true } });
    const count = ws.sent.length;
    ws.onmessage?.({
      data: JSON.stringify({
        op: 'error',
        id: '',
        error: 'not authorized: publishing events is outside this token scope',
      }),
    });
    ws.onmessage?.({
      data: JSON.stringify({ op: 'result', id: call.id, result: { retained: true } }),
    });
    expect(await pending).toEqual({ retained: true });
    vi.advanceTimersByTime(20000);
    expect(ws.sent.length).toBe(count);
    client.stop();
  });

  it('fires onReconnect on a reconnect but never on the first connect', () => {
    const client = new HubBusClient('tok');
    const onReconnect = vi.fn();
    client.onReconnect(onReconnect);
    client.start();

    // First connect — handlers must NOT fire.
    FakeWS.instances[0].open();
    expect(onReconnect).not.toHaveBeenCalled();

    // Socket drops; backoff timer schedules a reconnect.
    FakeWS.instances[0].die();
    vi.advanceTimersByTime(600);
    expect(FakeWS.instances.length).toBe(2);

    // Second connect IS a reconnect — handlers fire exactly once.
    FakeWS.instances[1].open();
    expect(onReconnect).toHaveBeenCalledTimes(1);

    client.stop();
  });

  it('re-asserts active subscriptions after a reconnect', () => {
    const client = new HubBusClient('tok');
    client.start();
    FakeWS.instances[0].open();
    client.subscribe('agent.*', () => {});
    expect(FakeWS.instances[0].sent.some((f) => f.includes('"subscribe"'))).toBe(true);

    FakeWS.instances[0].die();
    vi.advanceTimersByTime(600);
    FakeWS.instances[1].open();
    // The new socket must re-send the subscription so events resume flowing.
    expect(
      FakeWS.instances[1].sent.some((f) => f.includes('"subscribe"') && f.includes('agent.*')),
    ).toBe(true);

    client.stop();
  });

  it('wakes a dead socket immediately when the page is shown again', () => {
    const client = new HubBusClient('tok');
    client.start();
    FakeWS.instances[0].open();

    // Socket silently dies in the background: readyState flips to CLOSED but no
    // reconnect timer has fired yet (timers are throttled while hidden).
    FakeWS.instances[0].readyState = FakeWS.CLOSED;

    // Returning to the tab must force a fresh connection without waiting on backoff.
    document.dispatchEvent(new Event('visibilitychange'));
    expect(FakeWS.instances.length).toBe(2);

    client.stop();
  });

  it('does not reopen after an auth rejection on subsequent wake events', () => {
    const client = new HubBusClient('bad-tok');
    client.start();
    FakeWS.instances[0].open();

    // Server rejects the token and closes the socket with an auth-rejection code.
    // No reconnect should be scheduled (bad token won't get better on retry).
    FakeWS.instances[0].die(1008);
    vi.advanceTimersByTime(600);
    expect(FakeWS.instances.length).toBe(1);

    // User tabs away and back: the socket is CLOSED (not live), but the rejection
    // must be remembered so wake() does NOT spawn another doomed bad-token connection.
    document.dispatchEvent(new Event('visibilitychange'));
    expect(FakeWS.instances.length).toBe(1);

    // Same for a network-restore event.
    window.dispatchEvent(new Event('online'));
    expect(FakeWS.instances.length).toBe(1);

    client.stop();
  });

  it('does not reconnect on wake when the socket is healthy', () => {
    const client = new HubBusClient('tok');
    client.start();
    FakeWS.instances[0].open();
    // Fresh activity stamp from the open; a still-live socket should be left alone.
    window.dispatchEvent(new Event('online'));
    expect(FakeWS.instances.length).toBe(1);

    client.stop();
  });
});

describe('HubBusClient send queue', () => {
  beforeEach(() => {
    FakeWS.instances = [];
    vi.stubGlobal('WebSocket', FakeWS as unknown as typeof WebSocket);
    const storage = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value);
      },
      removeItem: (key: string) => {
        storage.delete(key);
      },
    });
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  const methodsSent = (ws: FakeWS): string[] =>
    ws.sent
      .map((f) => JSON.parse(f))
      .filter((f) => f.op === 'call')
      .map((f) => f.method);

  it('flushes calls queued before the socket opened', async () => {
    const client = new HubBusClient('tok');
    client.start();
    const pending = client.call('sessions.list');
    FakeWS.instances[0].open();
    expect(methodsSent(FakeWS.instances[0])).toEqual(['sessions.list']);

    // Keep the rejection handled; the test ends before any reply arrives.
    pending.catch(() => {});
    client.stop();
  });

  it('does not replay a queued call whose caller already timed out', async () => {
    // The caller has surfaced the failure by now — ClaudePane retracts the
    // optimistic bubble and hands the text back — so sending it later delivers
    // a message the user believes never went, and again when they retype it.
    const client = new HubBusClient('tok');
    client.start();
    const abandoned = client.call('claude.send');
    const rejected = expect(abandoned).rejects.toThrow(/timeout/);
    await vi.advanceTimersByTimeAsync(15_001);
    await rejected;

    FakeWS.instances[0].open();
    expect(methodsSent(FakeWS.instances[0])).toEqual([]);

    client.stop();
  });

  it('rejects the oldest queued calls instead of growing without bound', async () => {
    const client = new HubBusClient('tok');
    client.start();
    const first = client.call('first');
    const overflowed = expect(first).rejects.toThrow(/overflow/);
    // 200 is the cap; one more call must push `first` out.
    for (let i = 0; i < 200; i++) client.call(`filler${i}`).catch(() => {});
    await overflowed;

    FakeWS.instances[0].open();
    expect(methodsSent(FakeWS.instances[0])).not.toContain('first');
    expect(methodsSent(FakeWS.instances[0]).length).toBe(200);

    client.stop();
  });
});
