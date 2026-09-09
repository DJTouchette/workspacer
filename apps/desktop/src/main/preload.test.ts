/**
 * Regression tests for the MessagePort plumbing in preload.ts:
 *  - A late-arriving port on a *cancelled* subscription must NOT close the
 *    shared cached port (that port is also used by claudeWrite and other
 *    subscribers — closing it kills the session's I/O).
 *  - A getPort() timeout rejection must be handled (no unhandled rejection).
 *
 * Plus the webUtils bridge, which has to live here (webUtils is renderer-side)
 * and is the renderer's only way to turn a dropped File into a host path.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { IPC } from './shared/ipcChannels';

// ── Capture what preload exposes / registers ──────────────────────────────
const captured = vi.hoisted(() => ({
  api: undefined as any,
  ipcHandlers: {} as Record<string, (...args: any[]) => void>,
  getPathForFile: vi.fn((_file: unknown) => ''),
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
  webUtils: {
    getPathForFile: (file: unknown) => captured.getPathForFile(file),
  },
}));

class FakePort {
  closed = false;
  posted: any[] = [];
  listeners: Array<[string, (e: any) => void]> = [];
  started = false;
  start() {
    this.started = true;
  }
  close() {
    this.closed = true;
  }
  postMessage(d: any) {
    this.posted.push(d);
  }
  addEventListener(t: string, l: (e: any) => void) {
    this.listeners.push([t, l]);
  }
  removeEventListener(_t: string, l: (e: any) => void) {
    this.listeners = this.listeners.filter(([, ll]) => ll !== l);
  }
  emit(data: any) {
    for (const [, l] of this.listeners) l({ data });
  }
}

async function loadPreload() {
  captured.api = undefined;
  captured.ipcHandlers = {};
  vi.resetModules();
  await import('./preload');
  return captured.api;
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('preload webUtils bridge', () => {
  it('resolves a dropped File to its host path', async () => {
    captured.getPathForFile.mockReturnValue('/repo/shot.png');
    const api = await loadPreload();
    const file = { name: 'shot.png' };
    expect(api.getPathForFile(file)).toBe('/repo/shot.png');
    expect(captured.getPathForFile).toHaveBeenCalledWith(file);
  });

  it('returns an empty path instead of throwing when handed a non-File', async () => {
    captured.getPathForFile.mockImplementation(() => {
      throw new Error('not a File object'); // what webUtils does for a Blob/plain object
    });
    const api = await loadPreload();
    expect(api.getPathForFile({})).toBe('');
  });
});

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

it('bridges captured Fleet review selectors without adding filesystem or revision arguments', async () => {
  const api = await loadPreload();
  const { ipcRenderer } = await import('electron');
  const request = {
    ownerSessionId: 'manager',
    workerSessionId: 'worker',
    evidenceId: 'opaque',
    file: 'recorded.ts',
  };
  await api.dispatchHistoryRead();
  expect(ipcRenderer.invoke).toHaveBeenLastCalledWith(IPC.DISPATCH_HISTORY_READ);
  await api.fleetReviewRead(request);
  expect(ipcRenderer.invoke).toHaveBeenLastCalledWith(IPC.FLEET_REVIEW_READ, request);
  await api.fleetReviewForget(request);
  expect(ipcRenderer.invoke).toHaveBeenLastCalledWith(IPC.FLEET_REVIEW_FORGET, request);
});

it('exposes a read-only runtime status request through the declared IPC channel', async () => {
  const api = await loadPreload();
  const { ipcRenderer } = await import('electron');
  await api.agentRuntimeStatus();
  expect(ipcRenderer.invoke).toHaveBeenCalledWith(IPC.AGENT_RUNTIME_STATUS);
});

it('bridges only task-owned edit and open selectors', async () => {
  const api = await loadPreload();
  const { ipcRenderer } = await import('electron');
  const edit = {
    taskId: 'task',
    expectedTaskRevision: 2,
    action: 'waive' as const,
    stepId: 'review',
  };
  await api.taskInspectorEdit(edit);
  expect(ipcRenderer.invoke).toHaveBeenLastCalledWith(IPC.TASK_INSPECTOR_EDIT, edit);
  const open = { taskId: 'task', kind: 'worktree' as const, dispatchId: 'dispatch' };
  await api.taskInspectorOpen(open);
  expect(ipcRenderer.invoke).toHaveBeenLastCalledWith(IPC.TASK_INSPECTOR_OPEN, open);
});

it('provider readiness defaults to a free read; only explicit true requests a ping', async () => {
  const api = await loadPreload();
  const { ipcRenderer } = await import('electron');
  await api.providerReadiness('codex');
  expect(ipcRenderer.invoke).toHaveBeenLastCalledWith(IPC.PROVIDER_READINESS, 'codex', false);
  await api.providerReadiness('claude', true);
  expect(ipcRenderer.invoke).toHaveBeenLastCalledWith(IPC.PROVIDER_READINESS, 'claude', true);
});
