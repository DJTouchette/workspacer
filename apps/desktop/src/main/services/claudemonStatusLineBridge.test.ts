/**
 * Tests for claudemonStatusLineBridge — folds the daemon's /statusline/stream
 * SSE feed into claudeSessionStore.applyStatusLine.
 *
 * Load-bearing: the wire is snake_case and the store wants camelCase, so the
 * mapping is where a rename regression would bite. We verify the full field
 * remap, tolerance of a frame with no status_line object, malformed-JSON skip,
 * and the usual start/stop behaviour.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

let capturedOpts: any;
const consumeSseStream = vi.fn(async (_url: string, opts: any) => {
  capturedOpts = opts;
});
vi.mock('../lib/sseConsumer', () => ({ consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])) }));

const applyStatusLine = vi.fn();
vi.mock('./claudeSessionStore', () => ({ claudeSessionStore: { applyStatusLine: (...a: unknown[]) => applyStatusLine(...a) } }));

vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://daemon' }));

const { startClaudemonStatusLineBridge, stopClaudemonStatusLineBridge } = await import('./claudemonStatusLineBridge');

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  capturedOpts = undefined;
  stopClaudemonStatusLineBridge();
});

describe('claudemonStatusLineBridge', () => {
  it('subscribes to the daemon /statusline/stream endpoint', async () => {
    await startClaudemonStatusLineBridge();
    expect(consumeSseStream.mock.calls[0][0]).toBe('http://daemon/statusline/stream');
  });

  it('remaps the snake_case wire status_line to the camelCase store shape', async () => {
    await startClaudemonStatusLineBridge();
    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 's1',
        status_line: {
          model_display: 'Opus',
          context_used_pct: 42,
          context_window_size: 200000,
          total_input_tokens: 100,
          total_output_tokens: 200,
          cost_usd: 1.5,
          five_hour_pct: 10,
          five_hour_resets_at: 't1',
          seven_day_pct: 20,
          seven_day_resets_at: 't2',
          received_at: 't3',
        },
      }),
    );

    expect(applyStatusLine).toHaveBeenCalledTimes(1);
    const [id, sl] = applyStatusLine.mock.calls[0];
    expect(id).toBe('s1');
    expect(sl).toEqual({
      modelDisplay: 'Opus',
      contextUsedPct: 42,
      contextWindowSize: 200000,
      totalInputTokens: 100,
      totalOutputTokens: 200,
      costUSD: 1.5,
      fiveHourPct: 10,
      fiveHourResetsAt: 't1',
      sevenDayPct: 20,
      sevenDayResetsAt: 't2',
      receivedAt: 't3',
    });
  });

  it('tolerates a frame with no status_line object (all fields undefined)', async () => {
    await startClaudemonStatusLineBridge();
    expect(() => capturedOpts.onFrame(JSON.stringify({ session_id: 's1' }))).not.toThrow();
    const [id, sl] = applyStatusLine.mock.calls[0];
    expect(id).toBe('s1');
    expect(sl.modelDisplay).toBeUndefined();
  });

  it('skips malformed JSON without calling the store', async () => {
    await startClaudemonStatusLineBridge();
    expect(() => capturedOpts.onFrame('nope')).not.toThrow();
    expect(applyStatusLine).not.toHaveBeenCalled();
  });

  it('onError does not throw', async () => {
    await startClaudemonStatusLineBridge();
    expect(() => capturedOpts.onError(new Error('reset'))).not.toThrow();
  });

  it('is idempotent and restartable', async () => {
    await startClaudemonStatusLineBridge();
    await startClaudemonStatusLineBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(1);
    stopClaudemonStatusLineBridge();
    expect(capturedOpts.signal.aborted).toBe(true);
    await startClaudemonStatusLineBridge();
    expect(consumeSseStream).toHaveBeenCalledTimes(2);
  });
});
