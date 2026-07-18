/**
 * Tests for claudemonEventBridge — folds a MANAGED session's mode from the
 * daemon's /events SSE feed into claudeSessionStore.applyManagedMode.
 *
 * The load-bearing behaviour: only frames with event === 'Managed' and a string
 * mode drive the store; Spawn / SessionEnd / Claude-PTY updates must be ignored
 * (a Claude session's ambientState is hook-driven and must not be clobbered
 * here). Malformed JSON is skipped, and start/stop behave like the other bridges.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

let capturedOpts: any;
const consumeSseStream = vi.fn(async (_url: string, opts: any) => {
  capturedOpts = opts;
});
vi.mock('../lib/sseConsumer', () => ({
  consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])),
}));

const applyManagedMode = vi.fn();
const handleHookEvent = vi.fn();
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    applyManagedMode: (...a: unknown[]) => applyManagedMode(...a),
    handleHookEvent: (...a: unknown[]) => handleHookEvent(...a),
  },
}));

vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://daemon' }));

const { startClaudemonEventBridge, stopClaudemonEventBridge } =
  await import('./claudemonEventBridge');

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  capturedOpts = undefined;
  stopClaudemonEventBridge();
});

describe('claudemonEventBridge', () => {
  it('subscribes to the daemon /events endpoint', async () => {
    await startClaudemonEventBridge();
    expect(consumeSseStream.mock.calls[0][0]).toBe('http://daemon/events');
  });

  it('applies a Managed mode change to the store', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(
      JSON.stringify({ event: 'Managed', session_id: 's1', state: { mode: 'working' } }),
    );
    expect(applyManagedMode).toHaveBeenCalledWith('s1', 'working', {
      provider: undefined,
      transport: undefined,
      pending: null,
    });
  });

  it('forwards the backend identity (provider/transport) from the state frame', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(
      JSON.stringify({
        event: 'Managed',
        session_id: 's1',
        state: { mode: 'responding', provider: 'claude', transport: 'stream' },
      }),
    );
    expect(applyManagedMode).toHaveBeenCalledWith('s1', 'responding', {
      provider: 'claude',
      transport: 'stream',
      pending: null,
    });
  });

  it('forwards the pending approval payload from the state frame', async () => {
    await startClaudemonEventBridge();
    const pending = {
      kind: 'approval',
      tool: 'exec_command',
      summary: 'npm test',
      raw: { command: ['npm', 'test'] },
    };
    capturedOpts.onFrame(
      JSON.stringify({
        event: 'Managed',
        session_id: 's1',
        state: { mode: 'approval', provider: 'codex', pending },
      }),
    );
    expect(applyManagedMode).toHaveBeenCalledWith('s1', 'approval', {
      provider: 'codex',
      transport: undefined,
      pending,
    });
  });

  it('ignores non-Managed mode-change events (Spawn / Claude-PTY updates)', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(
      JSON.stringify({ event: 'Spawn', session_id: 's1', state: { mode: 'working' } }),
    );
    expect(applyManagedMode).not.toHaveBeenCalled();
  });

  it('routes a managed SessionEnd frame into the store so the session ends', async () => {
    await startClaudemonEventBridge();
    // claudemon's deregister_managed broadcasts this when a managed (Codex /
    // OpenCode / Pi / Claude-stream) session's process exits. Managed backends
    // fire no Claude hooks, so this is the ONLY signal that ends them.
    capturedOpts.onFrame(
      JSON.stringify({ event: 'SessionEnd', session_id: 's1', state: { mode: 'stopped' } }),
    );
    // It must be forwarded to the store's ended pipeline (status -> 'ended',
    // history write, per-session eviction). Before the fix the frame was
    // dropped, so handleHookEvent was never called and this expectation fails.
    expect(handleHookEvent).toHaveBeenCalledWith({
      hook_event_name: 'SessionEnd',
      session_id: 's1',
    });
    // And it must NOT be misrouted as an ambient-mode change.
    expect(applyManagedMode).not.toHaveBeenCalled();
  });

  it('ignores a Managed frame with a missing/non-string mode', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(JSON.stringify({ event: 'Managed', session_id: 's1', state: {} }));
    capturedOpts.onFrame(JSON.stringify({ event: 'Managed', session_id: 's1' }));
    expect(applyManagedMode).not.toHaveBeenCalled();
  });

  it('ignores a Managed frame with no session_id', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(JSON.stringify({ event: 'Managed', state: { mode: 'working' } }));
    expect(applyManagedMode).not.toHaveBeenCalled();
  });

  it('skips malformed JSON without throwing', async () => {
    await startClaudemonEventBridge();
    expect(() => capturedOpts.onFrame('not json')).not.toThrow();
    expect(applyManagedMode).not.toHaveBeenCalled();
  });

  it('onError does not throw', async () => {
    await startClaudemonEventBridge();
    expect(() => capturedOpts.onError(new Error('reset'))).not.toThrow();
  });

  it('is idempotent and restartable', async () => {
    await startClaudemonEventBridge();
    await startClaudemonEventBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(1);
    stopClaudemonEventBridge();
    expect(capturedOpts.signal.aborted).toBe(true);
    await startClaudemonEventBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(2);
  });
});
