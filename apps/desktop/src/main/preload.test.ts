/**
 * Regression tests for the MessagePort plumbing in preload.ts:
 *  - A late-arriving port on a *cancelled* subscription must NOT close the
 *    shared cached port (that port is also used by claudeWrite and other
 *    subscribers — closing it kills the session's I/O).
 *  - A getPort() timeout rejection must be handled (no unhandled rejection).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { IPC } from './shared/ipcChannels';

// ── Capture what preload exposes / registers ──────────────────────────────
const captured = vi.hoisted(() => ({
  api: undefined as any,
  ipcHandlers: {} as Record<string, (...args: any[]) => void>,
}));

vi.mock('electron', () => ({
  contextBridge: {
    exposeInMainWorld: (_key: string, api: any) => {
      captured.api = api;
    },
  },
  ipcRenderer: {
    on: (channel: string, handler: (...args: any[]) => void) => {
      captured.ipcHandlers[channel] = handler;
    },
    send: vi.fn(),
    invoke: vi.fn(() => Promise.resolve()),
  },
}));

class FakePort {
  closed = false;
  posted: any[] = [];
  listeners: Array<[string, (e: any) => void]> = [];
  started = false;
  start() { this.started = true; }
  close() { this.closed = true; }
  postMessage(d: any) { this.posted.push(d); }
  addEventListener(t: string, l: (e: any) => void) { this.listeners.push([t, l]); }
  removeEventListener(_t: string, l: (e: any) => void) {
    this.listeners = this.listeners.filter(([, ll]) => ll !== l);
  }
  emit(data: any) { for (const [, l] of this.listeners) l({ data }); }
}

async function loadPreload() {
  captured.api = undefined;
  captured.ipcHandlers = {};
  vi.resetModules();
  await import('./preload');
  return captured.api;
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('preload MessagePort plumbing', () => {
  beforeEach(() => {
    vi.useRealTimers();
  });

  it('does not close the shared cached port when a subscription is cancelled before the port arrives', async () => {
    const api = await loadPreload();
    const port = new FakePort();

    // Subscribe, then cancel BEFORE the port is delivered.
    const unsub = api.onClaudeOutput('sess1', () => {});
    unsub();

    // Now the port arrives late and resolves the pending waiter.
    captured.ipcHandlers[IPC.CLAUDE_PORT]({ ports: [port] }, { sessionId: 'sess1' });
    await tick();

    // The cached port must survive — claudeWrite and re-subscribers depend on it.
    expect(port.closed).toBe(false);

    // claudeWrite still reaches the live port.
    api.claudeWrite('sess1', 'hello');
    expect(port.posted).toContain('hello');
  });

  it('delivers data to an active subscriber', async () => {
    const api = await loadPreload();
    const port = new FakePort();
    const got: string[] = [];
    api.onClaudeOutput('sess2', (d: string) => got.push(d));
    captured.ipcHandlers[IPC.CLAUDE_PORT]({ ports: [port] }, { sessionId: 'sess2' });
    await tick();
    port.emit('chunk');
    expect(got).toEqual(['chunk']);
  });

  it('handles a getPort timeout without an unhandled rejection', async () => {
    vi.useFakeTimers();
    const rejections: unknown[] = [];
    const onRej = (e: unknown) => rejections.push(e);
    process.on('unhandledRejection', onRej);

    const api = await loadPreload();
    // Subscribe but never deliver a port → the 10s timeout fires and rejects.
    api.onClaudeOutput('never', () => {});
    await vi.advanceTimersByTimeAsync(11_000);

    // flush microtasks so any unhandled rejection would surface
    vi.useRealTimers();
    await tick();
    process.off('unhandledRejection', onRej);
    expect(rejections).toEqual([]);
  });
});
