/**
 * Rate-limit warning dedup — regression test.
 *
 * The account-global "approaching a usage limit" alert must fire once per
 * episode. The dedup latch (lastWarnedWindow) must NOT be reset by a
 * warning-less frame that still reports high utilization: interactive (PTY)
 * sessions never carry a rate_limit_warning, and the periodic account-usage
 * re-push is warning-less too, even at 85%. Only a genuine drop back under the
 * daemon's warning threshold should re-arm the alert.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const notificationShow = vi.fn();
const notifySystem = vi.fn();

let capturedOpts: any;
const consumeSseStream = vi.fn(async (_url: string, opts: any) => {
  capturedOpts = opts;
});
vi.mock('../lib/sseConsumer', () => ({
  consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])),
}));

vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { applyStatusLine: () => {} },
}));

vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://daemon' }));

vi.mock('electron', () => ({
  Notification: class {
    static isSupported() {
      return true;
    }
    constructor(_opts: any) {}
    show() {
      notificationShow();
    }
  },
}));

vi.mock('./configService', () => ({
  configService: { getConfig: () => ({ notifications: { enabled: true } }) },
}));

vi.mock('./systemNotice', () => ({
  notifySystem: (...a: unknown[]) => notifySystem(...a),
}));

const { startClaudemonStatusLineBridge, stopClaudemonStatusLineBridge } = await import(
  './claudemonStatusLineBridge'
);

beforeEach(() => {
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  notificationShow.mockClear();
  notifySystem.mockClear();
  capturedOpts = undefined;
  stopClaudemonStatusLineBridge();
});

describe('claudemonStatusLineBridge rate-limit dedup', () => {
  it('does not re-fire when a warning-less PTY frame arrives at still-high utilization', async () => {
    await startClaudemonStatusLineBridge();
    const warn5h = "You're close to your 5-hour usage limit — 85% used";

    // 1) Stream-transport session emits the account-global warning at 85%.
    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'stream1',
        status_line: { rate_limit_warning: warn5h, five_hour_pct: 85 },
      }),
    );

    // 2) A concurrent PTY session (and the periodic account re-push) reports the
    //    gauge at 85% but carries NO warning — the account is NOT comfortable.
    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'pty1',
        status_line: { five_hour_pct: 85 },
      }),
    );

    // 3) The stream session ticks again with the same 5h warning.
    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'stream1',
        status_line: { rate_limit_warning: warn5h, five_hour_pct: 85 },
      }),
    );

    // The alert must have fired exactly once for this episode, not once per tick.
    expect(notifySystem).toHaveBeenCalledTimes(1);
    expect(notificationShow).toHaveBeenCalledTimes(1);
  });
});
