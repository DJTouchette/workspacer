/**
 * Tests for claudemonHookBridge — folds the daemon's /hooks/stream SSE feed into
 * claudeSessionStore.handleHookEvent.
 *
 * The bridge is a thin wrapper over the shared SSE consumer, so we mock
 * ../lib/sseConsumer to capture the options object and then drive its onFrame /
 * onError callbacks directly. What matters:
 *   - the wire `event` field is translated to `hook_event_name` (the store reads
 *     `hook_event_name ?? type`), with the flattened payload preserved;
 *   - malformed JSON is skipped, never killing the stream;
 *   - a store that throws on a frame does not propagate out of onFrame;
 *   - start() is idempotent and stop() aborts the consumer's signal.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

let capturedOpts: any;
const consumeSseStream = vi.fn(async (_url: string, opts: any) => {
  capturedOpts = opts;
});
vi.mock('../lib/sseConsumer', () => ({
  consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])),
}));

const handleHookEvent = vi.fn();
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { handleHookEvent: (...a: unknown[]) => handleHookEvent(...a) },
}));

vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://daemon' }));

const { startClaudemonHookBridge, stopClaudemonHookBridge } = await import('./claudemonHookBridge');

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  capturedOpts = undefined;
  stopClaudemonHookBridge();
});

describe('claudemonHookBridge', () => {
  it('subscribes to the daemon /hooks/stream endpoint', async () => {
    await startClaudemonHookBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(1);
    expect(consumeSseStream.mock.calls[0][0]).toBe('http://daemon/hooks/stream');
  });

  it('translates wire `event` to `hook_event_name` and forwards the flattened payload', async () => {
    await startClaudemonHookBridge();
    capturedOpts.onFrame(
      JSON.stringify({ event: 'PreToolUse', session_id: 's1', tool_name: 'Bash' }),
    );

    expect(handleHookEvent).toHaveBeenCalledTimes(1);
    const arg = handleHookEvent.mock.calls[0][0];
    expect(arg.hook_event_name).toBe('PreToolUse');
    expect(arg.session_id).toBe('s1');
    expect(arg.tool_name).toBe('Bash');
  });

  it('skips malformed JSON without calling the store or throwing', async () => {
    await startClaudemonHookBridge();
    expect(() => capturedOpts.onFrame('{ not json')).not.toThrow();
    expect(handleHookEvent).not.toHaveBeenCalled();
  });

  it('swallows a store error so one bad frame does not kill the stream', async () => {
    handleHookEvent.mockImplementationOnce(() => {
      throw new Error('store boom');
    });
    await startClaudemonHookBridge();
    expect(() =>
      capturedOpts.onFrame(JSON.stringify({ event: 'Stop', session_id: 's1' })),
    ).not.toThrow();
  });

  it('onError does not throw', async () => {
    await startClaudemonHookBridge();
    expect(() => capturedOpts.onError(new Error('stream reset'))).not.toThrow();
  });

  it('is idempotent — a second start does not open a second stream', async () => {
    await startClaudemonHookBridge();
    await startClaudemonHookBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(1);
  });

  it('stop() aborts the consumer signal', async () => {
    await startClaudemonHookBridge();
    expect(capturedOpts.signal.aborted).toBe(false);
    stopClaudemonHookBridge();
    expect(capturedOpts.signal.aborted).toBe(true);
  });

  it('can restart after stop', async () => {
    await startClaudemonHookBridge();
    stopClaudemonHookBridge();
    await startClaudemonHookBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(2);
  });
});
