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
vi.mock('../lib/sseConsumer', () => ({ consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])) }));

const applyManagedMode = vi.fn();
vi.mock('./claudeSessionStore', () => ({ claudeSessionStore: { applyManagedMode: (...a: unknown[]) => applyManagedMode(...a) } }));

vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://daemon' }));

const { startClaudemonEventBridge, stopClaudemonEventBridge } = await import('./claudemonEventBridge');

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
    capturedOpts.onFrame(JSON.stringify({ event: 'Managed', session_id: 's1', state: { mode: 'working' } }));
    expect(applyManagedMode).toHaveBeenCalledWith('s1', 'working');
  });

  it('ignores non-Managed events (Spawn / SessionEnd / Claude-PTY updates)', async () => {
    await startClaudemonEventBridge();
    capturedOpts.onFrame(JSON.stringify({ event: 'Spawn', session_id: 's1', state: { mode: 'working' } }));
    capturedOpts.onFrame(JSON.stringify({ event: 'SessionEnd', session_id: 's1' }));
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
