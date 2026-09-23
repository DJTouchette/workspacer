import { readFileSync } from 'node:fs';
/**
 * Tests for claudemonEventBridge — folds a MANAGED session's mode from the
 * daemon's /events SSE feed into claudeSessionStore.applyManagedMode.
 *
 * The load-bearing behaviour: only frames with event === 'Managed' and a string
 * mode drive the store; Spawn / SessionEnd / Claude-PTY updates must be ignored
 * (a Claude session's ambientState is hook-driven and must not be clobbered
 * here). Malformed JSON is skipped, and start/stop behave like the other bridges.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

let capturedOpts: any;
const consumeSseStream = vi.fn(async (_url: string, opts: any) => {
  capturedOpts = opts;
});
vi.mock('../lib/sseConsumer', () => ({
  consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])),
}));

const applyExecutionEngine = vi.fn();
const applyManagedMode = vi.fn();
const handleHookEvent = vi.fn();
const getSnapshot = vi.fn();
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    applyManagedMode: (...a: unknown[]) => applyManagedMode(...a),
    applyExecutionEngine: (...a: unknown[]) => applyExecutionEngine(...a),
    handleHookEvent: (...a: unknown[]) => handleHookEvent(...a),
    getSnapshot: (...a: unknown[]) => getSnapshot(...a),
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
  getSnapshot.mockReturnValue({ status: 'active' });
});

afterEach(() => {
  stopClaudemonEventBridge();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('claudemonEventBridge', () => {
  it('reconciles a lost final exit on connect and every reconnect', async () => {
    const fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [{ session_id: 's1', mode: 'stopped', provider: 'codex' }],
    });
    vi.stubGlobal('fetch', fetch);
    await startClaudemonEventBridge();
    capturedOpts.onConnect();
    await vi.waitFor(() => expect(handleHookEvent).toHaveBeenCalledTimes(1));
    capturedOpts.onConnect();
    await vi.waitFor(() => expect(handleHookEvent).toHaveBeenCalledTimes(2));
    expect(handleHookEvent).toHaveBeenLastCalledWith({
      hook_event_name: 'SessionEnd',
      session_id: 's1',
    });
    expect(fetch).toHaveBeenCalledWith(
      'http://daemon/sessions?include_archived=true&include_empty=true&state_only=true',
      expect.anything(),
    );
  });

  it('recovers pending decisions on lag without touching PTY, remote or unknown rows', async () => {
    const pending = { kind: 'approval', tool: 'shell', raw: {} };
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => [
          { session_id: 's1', provider: 'claude', transport: 'stream', mode: 'approval', pending },
          { session_id: 'pty', provider: 'claude', transport: 'pty', mode: 'responding' },
          { session_id: 'remote', provider: 'codex', mode: 'stopped' },
          { session_id: 'unknown', provider: 'codex', mode: 'stopped' },
        ],
      }),
    );
    getSnapshot.mockImplementation((id) =>
      id === 'unknown' ? null : { status: 'active', hub: id === 'remote' ? 'peer' : undefined },
    );
    await startClaudemonEventBridge();
    capturedOpts.onFrame(JSON.stringify({ event: 'Resync', reason: 'lagged' }));
    await vi.waitFor(() => expect(applyManagedMode).toHaveBeenCalledTimes(1));
    expect(applyManagedMode).toHaveBeenCalledWith(
      's1',
      'approval',
      expect.objectContaining({ pending }),
    );
    expect(handleHookEvent).not.toHaveBeenCalled();
  });

  it('does not overwrite live frames with an older in-flight reconciliation', async () => {
    let finish!: (value: unknown) => void;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      ),
    );
    await startClaudemonEventBridge();
    capturedOpts.onConnect();
    capturedOpts.onFrame(
      JSON.stringify({
        event: 'Managed',
        session_id: 's1',
        state: { mode: 'responding', provider: 'codex' },
      }),
    );
    finish({
      ok: true,
      json: async () => [{ session_id: 's1', mode: 'stopped', provider: 'codex' }],
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(handleHookEvent).not.toHaveBeenCalled();
    expect(applyManagedMode).toHaveBeenCalledTimes(1);
  });

  it('does not let queued pre-lag frames hide a newer terminal snapshot', async () => {
    let finish!: (value: unknown) => void;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      ),
    );
    await startClaudemonEventBridge();
    capturedOpts.onFrame('{"event":"Resync"}');
    const queued = {
      event: 'Managed',
      session_id: 's1',
      state: {
        mode: 'responding',
        provider: 'codex',
        updated_at: '2026-09-23T12:00:00.000000001Z',
      },
    };
    capturedOpts.onFrame(JSON.stringify(queued));
    finish({
      ok: true,
      json: async () => [
        {
          session_id: 's1',
          mode: 'stopped',
          provider: 'codex',
          updated_at: '2026-09-23T12:00:00.000000002Z',
        },
      ],
    });
    await vi.waitFor(() => expect(handleHookEvent).toHaveBeenCalledTimes(1));
    // A still-queued frame arriving after the snapshot cannot resurrect it.
    capturedOpts.onFrame(JSON.stringify(queued));
    expect(applyManagedMode).toHaveBeenCalledTimes(1);
  });

  it('coalesces repeated lag recovery and cancels an in-flight result on stop', async () => {
    let finish!: (value: unknown) => void;
    const fetch = vi.fn().mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    vi.stubGlobal('fetch', fetch);
    await startClaudemonEventBridge();
    capturedOpts.onConnect();
    capturedOpts.onFrame('{"event":"Resync"}');
    capturedOpts.onFrame('{"event":"Resync"}');
    expect(fetch).toHaveBeenCalledTimes(1);
    stopClaudemonEventBridge();
    finish({
      ok: true,
      json: async () => [{ session_id: 's1', mode: 'stopped', provider: 'codex' }],
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(handleHookEvent).not.toHaveBeenCalled();
  });

  it('retries failed state recovery even when no further events arrive', async () => {
    vi.useFakeTimers();
    const fetch = vi
      .fn()
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValue({
        ok: true,
        json: async () => [{ session_id: 's1', mode: 'stopped', provider: 'codex' }],
      });
    vi.stubGlobal('fetch', fetch);
    await startClaudemonEventBridge();
    capturedOpts.onConnect();
    await vi.advanceTimersByTimeAsync(1000);
    expect(handleHookEvent).toHaveBeenCalledTimes(1);
  });

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
      backgroundTasks: undefined,
      subagents: undefined,
      selection: {},
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
      backgroundTasks: undefined,
      subagents: undefined,
      selection: {},
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
      backgroundTasks: undefined,
      subagents: undefined,
      selection: {},
    });
  });

  it('forwards managed subagent rows and background count from the state frame', async () => {
    await startClaudemonEventBridge();
    const subagents = [
      {
        id: 'child-1',
        type: 'codex',
        status: 'running',
        startedAt: 1000,
        description: 'inspect',
        toolUseId: 'call-1',
      },
    ];
    capturedOpts.onFrame(
      JSON.stringify({
        event: 'Managed',
        session_id: 's1',
        state: { mode: 'input', provider: 'codex', background_tasks: 1, subagents },
      }),
    );
    expect(applyManagedMode).toHaveBeenCalledWith('s1', 'input', {
      provider: 'codex',
      transport: undefined,
      pending: null,
      backgroundTasks: 1,
      subagents,
      selection: {},
    });
  });

  // claudemon OWNS the canonical selection slice and this feed is the only
  // channel that carries it live. The bridge maps its spelling and forwards it;
  // a frame that says nothing forwards an empty slice, which the store reads as
  // "leave the row's own values alone".
  it('maps the daemon selection slice to camelCase', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(
      JSON.stringify({
        event: 'Managed',
        session_id: 's1',
        state: {
          mode: 'responding',
          requested_selection: { model: 'claude-opus-5', context_window: 1_000_000 },
          resolved_context_window: 1_000_000,
        },
      }),
    );
    expect(applyManagedMode.mock.calls[0][2].selection).toEqual({
      requestedSelection: { model: 'claude-opus-5', contextWindow: 1_000_000 },
      resolvedContextWindow: 1_000_000,
    });
  });

  it('forwards an empty slice for a frame that carries neither field', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(
      JSON.stringify({ event: 'Managed', session_id: 's1', state: { mode: 'input' } }),
    );
    expect(applyManagedMode.mock.calls[0][2].selection).toEqual({});
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

it('projects the read-only execution pin without deriving readiness from provider', async () => {
  await startClaudemonEventBridge();
  const executionEngine = JSON.parse(
    readFileSync(
      new URL('../../../../../contracts/execution-engine-v1.json', import.meta.url),
      'utf8',
    ),
  ).metadata;
  capturedOpts.onFrame(
    JSON.stringify({
      event: 'Managed',
      session_id: 'engine',
      state: { mode: 'stopped', provider: 'claude', execution_engine: executionEngine },
    }),
  );
  expect(applyManagedMode).toHaveBeenCalledWith(
    'engine',
    'stopped',
    expect.objectContaining({ executionEngine }),
  );
});

it('drops a stale engine terminal before it can wake or end the successor', async () => {
  await startClaudemonEventBridge();
  applyExecutionEngine.mockReturnValueOnce(false);
  capturedOpts.onFrame(
    JSON.stringify({
      event: 'SessionEnd',
      session_id: 's1',
      state: {
        mode: 'stopped',
        execution_engine: {
          id: 'claudemon-v1',
          api_version: 1,
          implementation_version: '1',
          generation: 1,
          readiness: 'ready',
        },
      },
    }),
  );
  expect(handleHookEvent).not.toHaveBeenCalled();
});
