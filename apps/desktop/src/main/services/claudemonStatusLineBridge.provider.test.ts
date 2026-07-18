/**
 * Rate-limit warning dedup is PER-PROVIDER — regression test.
 *
 * The `/statusline/stream` feed carries ticks for every provider (Claude and
 * Codex sessions both flow through this one bridge and both synthesize the
 * identical "You're close to your 5-hour usage limit …" message). But rate-limit
 * windows are per-ACCOUNT, and Claude vs Codex are DISTINCT accounts. The dedup
 * latch must therefore be keyed by provider × window, not window alone —
 * otherwise once Claude warns on its 5h window, Codex's own 5h warning is
 * swallowed and the user is never alerted about the Codex account.
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

// Map session_id → provider so the bridge can tell the two accounts apart.
const providerBySession: Record<string, string> = {
  claude1: 'claude',
  codex1: 'codex',
};
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    applyStatusLine: () => {},
    providerOf: (sid: string) => providerBySession[sid],
  },
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

const { startClaudemonStatusLineBridge, stopClaudemonStatusLineBridge } =
  await import('./claudemonStatusLineBridge');

beforeEach(() => {
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  notificationShow.mockClear();
  notifySystem.mockClear();
  capturedOpts = undefined;
  stopClaudemonStatusLineBridge();
});

describe('claudemonStatusLineBridge per-provider rate-limit dedup', () => {
  it('fires a separate warning for each provider on the same window type', async () => {
    await startClaudemonStatusLineBridge();
    const warn5h = "You're close to your 5-hour usage limit — 82% used";

    // Claude account crosses 80% on its 5h window → warning fires.
    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'claude1',
        status_line: { rate_limit_warning: warn5h, five_hour_pct: 82 },
      }),
    );

    // Shortly after, the Codex account crosses 80% on ITS OWN 5h window. Same
    // window type, different account — the user must still be alerted.
    capturedOpts.onFrame(
      JSON.stringify({
        session_id: 'codex1',
        status_line: { rate_limit_warning: warn5h, five_hour_pct: 82 },
      }),
    );

    // Two distinct accounts warned → two OS notifications, two banners.
    expect(notificationShow).toHaveBeenCalledTimes(2);
    expect(notifySystem).toHaveBeenCalledTimes(2);
  });
});
