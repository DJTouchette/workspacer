import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HubBusClient } from './hubBusClient';

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
  send(data: string): void { this.sent.push(data); }
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
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
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
    expect(FakeWS.instances[1].sent.some((f) => f.includes('"subscribe"') && f.includes('agent.*'))).toBe(true);

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
