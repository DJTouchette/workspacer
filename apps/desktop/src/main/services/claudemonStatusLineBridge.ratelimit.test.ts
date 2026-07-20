/**
 * Usage warnings must NOT interrupt — regression test.
 *
 * The bridge used to turn `rate_limit_warning` into an OS notification plus an
 * in-app banner at the daemon's 80% threshold, with a per-provider dedup latch
 * to stop it re-firing every tick. That whole path is gone: the per-window
 * gauges (5h / 7d / monthly `% used` + reset times) are a strictly more
 * accurate, always-visible signal. The warning survives only as passive text on
 * the session snapshot, which the Inspector renders.
 *
 * This test guards the removal — an alert re-added here would be an
 * interruption the gauges already cover.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const notificationShow = vi.fn();
const notifySystem = vi.fn();
const applyStatusLine = vi.fn();

let capturedOpts: any;
const consumeSseStream = vi.fn(async (_url: string, opts: any) => {
  capturedOpts = opts;
});
vi.mock('../lib/sseConsumer', () => ({
  consumeSseStream: (...a: unknown[]) => consumeSseStream(...(a as [string, any])),
}));

vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { applyStatusLine: (...a: unknown[]) => applyStatusLine(...a) },
}));

vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://daemon' }));

// Both alert channels stay mocked: the point of the test is that the bridge
// never reaches for either, not that they are unavailable.
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

const { startClaudemonStatusLineBridge, stopClaudemonStatusLineBridge } =
  await import('./claudemonStatusLineBridge');

beforeEach(() => {
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  notificationShow.mockClear();
  notifySystem.mockClear();
  applyStatusLine.mockClear();
  capturedOpts = undefined;
  stopClaudemonStatusLineBridge();
});

describe('claudemonStatusLineBridge usage warnings', () => {
  it('never raises an OS notification or in-app banner, even deep into a window', async () => {
    await startClaudemonStatusLineBridge();
    const warn5h = "You're close to your 5-hour usage limit — 85% used";

    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'stream1',
        status_line: { rate_limit_warning: warn5h, five_hour_pct: 85 },
      }),
    );
    // A second window warning on a different account would previously have been
    // a second alert; it must stay just as silent.
    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'codex1',
        status_line: { rate_limit_warning: 'Approaching your monthly limit', monthly_pct: 97 },
      }),
    );

    expect(notificationShow).not.toHaveBeenCalled();
    expect(notifySystem).not.toHaveBeenCalled();
  });

  it('still forwards the warning text onto the snapshot for passive display', async () => {
    await startClaudemonStatusLineBridge();
    const warn5h = "You're close to your 5-hour usage limit — 85% used";

    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'stream1',
        status_line: { rate_limit_warning: warn5h, five_hour_pct: 85 },
      }),
    );

    expect(applyStatusLine).toHaveBeenCalledTimes(1);
    const [sessionId, snapshot] = applyStatusLine.mock.calls[0];
    expect(sessionId).toBe('stream1');
    expect(snapshot.rateLimitWarning).toBe(warn5h);
    expect(snapshot.fiveHourPct).toBe(85);
  });
});
