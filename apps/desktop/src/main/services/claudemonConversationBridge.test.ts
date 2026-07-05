/**
 * Tests for claudemonConversationBridge — folds parsed conversation deltas from
 * the daemon's /conversation/stream SSE feed into
 * claudeSessionStore.applyConversationDelta.
 *
 * Load-bearing: each valid frame is handed to the store verbatim; malformed JSON
 * is skipped; a store that throws on a delta does not propagate out of onFrame
 * (one bad frame must not kill the stream). Start/stop mirror the other bridges.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

let capturedOpts: any;
const consumeSseStream = vi.fn(async (_url: string, opts: any) => {
  capturedOpts = opts;
});
vi.mock('../lib/sseConsumer', () => ({
  consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])),
}));

const applyConversationDelta = vi.fn();
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { applyConversationDelta: (...a: unknown[]) => applyConversationDelta(...a) },
}));

vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://daemon' }));

const { startClaudemonConversationBridge, stopClaudemonConversationBridge } =
  await import('./claudemonConversationBridge');

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  capturedOpts = undefined;
  stopClaudemonConversationBridge();
});

describe('claudemonConversationBridge', () => {
  it('subscribes to the daemon /conversation/stream endpoint', async () => {
    await startClaudemonConversationBridge();
    expect(consumeSseStream.mock.calls[0][0]).toBe('http://daemon/conversation/stream');
  });

  it('forwards a parsed delta to the store', async () => {
    await startClaudemonConversationBridge();
    const delta = { session_id: 's1', seq: 5, items: [{ kind: 'text' }] };
    capturedOpts.onFrame(JSON.stringify(delta));
    expect(applyConversationDelta).toHaveBeenCalledWith(delta);
  });

  it('skips malformed JSON without calling the store', async () => {
    await startClaudemonConversationBridge();
    expect(() => capturedOpts.onFrame('<<garbage')).not.toThrow();
    expect(applyConversationDelta).not.toHaveBeenCalled();
  });

  it('swallows a store error so a bad delta does not kill the stream', async () => {
    applyConversationDelta.mockImplementationOnce(() => {
      throw new Error('delta boom');
    });
    await startClaudemonConversationBridge();
    expect(() => capturedOpts.onFrame(JSON.stringify({ session_id: 's1' }))).not.toThrow();
  });

  it('onError does not throw', async () => {
    await startClaudemonConversationBridge();
    expect(() => capturedOpts.onError(new Error('reset'))).not.toThrow();
  });

  it('is idempotent and restartable', async () => {
    await startClaudemonConversationBridge();
    await startClaudemonConversationBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(1);
    stopClaudemonConversationBridge();
    expect(capturedOpts.signal.aborted).toBe(true);
    await startClaudemonConversationBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(2);
  });
});
