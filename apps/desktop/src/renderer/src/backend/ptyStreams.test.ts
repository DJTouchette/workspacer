/**
 * The hub's terminal attach/detach is per SESSION, but a session can have
 * several viewers — the pane that spawned it plus any watch pane opened on it
 * from the Agents or Fleet views, possibly in a different workspace where
 * pane-level dedupe can't see it. Getting the refcount wrong is invisible until
 * someone closes one pane and another pane's terminal silently goes dead, so
 * pin the lifetime rules here.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPtyStreams } from './webBackend';
import type { HubBusClient } from './hubBusClient';

type Handler = (ev: { data: unknown }) => void;

function fakeClient() {
  const calls: Array<{ method: string; params: any }> = [];
  const topics = new Map<string, Set<Handler>>();
  /** Reply to sessions.terminalKeepalive; default is a healthy lease. */
  let keepaliveReply: unknown = { ok: true };

  const client = {
    call: vi.fn((method: string, params: any) => {
      calls.push({ method, params });
      if (method === 'sessions.terminalKeepalive') return Promise.resolve(keepaliveReply);
      return Promise.resolve(undefined);
    }),
    subscribe: vi.fn((topic: string, handler: Handler) => {
      let set = topics.get(topic);
      if (!set) topics.set(topic, (set = new Set()));
      set.add(handler);
      return () => set!.delete(handler);
    }),
  };

  return {
    client: client as unknown as HubBusClient,
    calls,
    methods: () => calls.map((c) => c.method),
    countOf: (method: string) => calls.filter((c) => c.method === method).length,
    subscriberCount: (topic: string) => topics.get(topic)?.size ?? 0,
    emit: (topic: string, data: unknown) => {
      for (const h of topics.get(topic) ?? []) h({ data });
    },
    setKeepaliveReply: (reply: unknown) => {
      keepaliveReply = reply;
    },
  };
}

/** PTY bytes cross the bus base64-encoded; the stream decodes on the way out. */
const chunk = (text: string) => btoa(text);

describe('createPtyStreams', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('attaches once per viewer and delivers bytes to each', () => {
    const hub = fakeClient();
    const { stream } = createPtyStreams(hub.client);
    const a: string[] = [];
    const b: string[] = [];

    const closeA = stream('s1', (d) => a.push(d));
    const closeB = stream('s1', (d) => b.push(d));
    hub.emit('pty.bytes.s1', chunk('hello'));

    expect(a).toEqual(['hello']);
    expect(b).toEqual(['hello']);

    closeA();
    closeB();
  });

  it('does not detach the session while another viewer is still watching', () => {
    const hub = fakeClient();
    const { stream } = createPtyStreams(hub.client);
    const survivor: string[] = [];

    const closeWatcher = stream('s1', () => {});
    const closeOwner = stream('s1', (d) => survivor.push(d));

    closeWatcher();
    expect(hub.countOf('sessions.detachTerminal')).toBe(0);

    // The remaining viewer's stream must still be live — this is the whole bug:
    // one pane closing used to kill the forwarder for every other viewer.
    hub.emit('pty.bytes.s1', chunk('still here'));
    expect(survivor).toEqual(['still here']);

    closeOwner();
    expect(hub.countOf('sessions.detachTerminal')).toBe(1);
  });

  it('unsubscribes only the closing viewer', () => {
    const hub = fakeClient();
    const { stream } = createPtyStreams(hub.client);
    const closeA = stream('s1', () => {});
    stream('s1', () => {});
    expect(hub.subscriberCount('pty.bytes.s1')).toBe(2);
    closeA();
    expect(hub.subscriberCount('pty.bytes.s1')).toBe(1);
  });

  it('ignores a double teardown rather than dropping the refcount twice', () => {
    const hub = fakeClient();
    const { stream } = createPtyStreams(hub.client);
    const closeA = stream('s1', () => {});
    const closeB = stream('s1', () => {});

    closeA();
    closeA();
    expect(hub.countOf('sessions.detachTerminal')).toBe(0);

    closeB();
    expect(hub.countOf('sessions.detachTerminal')).toBe(1);
  });

  it('stops the keepalive once the last viewer leaves', async () => {
    const hub = fakeClient();
    const { stream } = createPtyStreams(hub.client);
    const close = stream('s1', () => {});
    await vi.advanceTimersByTimeAsync(10_000);
    expect(hub.countOf('sessions.terminalKeepalive')).toBe(1);

    close();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(hub.countOf('sessions.terminalKeepalive')).toBe(1);
  });

  it('re-attaches when the keepalive reports the lease already lapsed', async () => {
    // The socket stays healthy, so nothing fires onReconnect. Without honouring
    // this reply the stream is dead forever with no signal to the user.
    const hub = fakeClient();
    const { stream } = createPtyStreams(hub.client);
    const close = stream('s1', () => {});
    expect(hub.countOf('sessions.attachTerminal')).toBe(1);

    hub.setKeepaliveReply({ ok: false });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(hub.countOf('sessions.attachTerminal')).toBe(2);

    close();
  });

  it('leaves the stream alone while the lease is healthy', async () => {
    const hub = fakeClient();
    const { stream } = createPtyStreams(hub.client);
    const close = stream('s1', () => {});
    await vi.advanceTimersByTimeAsync(30_000);
    expect(hub.countOf('sessions.attachTerminal')).toBe(1);
    close();
  });

  it('fires every viewer re-prime hook on resize, debounced', async () => {
    const hub = fakeClient();
    const { stream, reprime } = createPtyStreams(hub.client);
    const closeA = stream('s1', () => {});
    const closeB = stream('s1', () => {});
    expect(hub.countOf('sessions.attachTerminal')).toBe(2);

    reprime('s1');
    reprime('s1'); // a burst of fit/resize events collapses to one replay each
    await vi.advanceTimersByTimeAsync(200);
    // Both viewers re-attached, once apiece — a Map keyed by sessionId used to
    // let the second viewer clobber the first's hook.
    expect(hub.countOf('sessions.attachTerminal')).toBe(4);

    closeA();
    closeB();
  });

  it('re-primes every live session on reconnect and nothing after teardown', async () => {
    const hub = fakeClient();
    const { stream, reprimeAll } = createPtyStreams(hub.client);
    const closeA = stream('s1', () => {});
    const closeB = stream('s2', () => {});
    expect(hub.countOf('sessions.attachTerminal')).toBe(2);

    reprimeAll();
    await vi.advanceTimersByTimeAsync(200);
    expect(hub.countOf('sessions.attachTerminal')).toBe(4);

    closeA();
    closeB();
    reprimeAll();
    await vi.advanceTimersByTimeAsync(200);
    expect(hub.countOf('sessions.attachTerminal')).toBe(4);
  });

  it('does not fire a pending re-prime after its viewer closed', async () => {
    const hub = fakeClient();
    const { stream, reprime } = createPtyStreams(hub.client);
    const close = stream('s1', () => {});
    reprime('s1');
    close(); // inside the 120ms debounce window
    await vi.advanceTimersByTimeAsync(200);
    expect(hub.countOf('sessions.attachTerminal')).toBe(1);
  });
});
