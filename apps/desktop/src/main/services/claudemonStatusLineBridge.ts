/**
 * Consumes the daemon's `/statusline/stream` SSE feed and forwards each tick
 * into `claudeSessionStore.applyStatusLine`. This is a separate channel from
 * `/hooks/stream` because the statusLine command fires very frequently — it is
 * deliberately kept off the hook fanout (and thus off the SQLite persistence
 * path). Reconnects with exponential backoff if the daemon restarts.
 */

import { Notification } from 'electron';
import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';
import { configService } from './configService';
import { notifySystem } from './systemNotice';

let abort: AbortController | null = null;

// Rate-limit warnings are account-global but arrive on every session's tick, so
// we dedup globally by which window is warning — a climbing % on the same window
// must not re-fire. Cleared when the account is comfortable again so the next
// episode re-alerts.
let lastWarnedWindow: string | null = null;

/** Coarse window key ('5h' | '7d' | 'monthly') from a warning message. */
function windowKeyOf(msg: string): string {
  if (msg.includes('7-day')) return '7d';
  if (msg.includes('monthly')) return 'monthly';
  return '5h';
}

/** Fire a rate-limit warning once per window-warning episode: an OS
 *  notification (respecting the master switch + sound) plus an in-app banner. */
function raiseRateLimitWarning(message: string | undefined): void {
  if (!message) {
    lastWarnedWindow = null; // comfortable again → allow the next episode to alert
    return;
  }
  const key = windowKeyOf(message);
  if (key === lastWarnedWindow) return; // same window still warning — no re-fire
  lastWarnedWindow = key;

  const cfg = (configService.getConfig() as any).notifications ?? {};
  const enabled = cfg.enabled !== false;
  if (enabled && Notification.isSupported()) {
    new Notification({
      title: 'Approaching a usage limit',
      body: message,
      silent: cfg.sound !== true,
    }).show();
  }
  // The in-app banner always shows (it's the app's own surface, not an OS
  // notification); a stable key means a later window replaces rather than stacks.
  notifySystem({ level: 'warn', key: 'rate-limit-warning', title: message });
}

export async function startClaudemonStatusLineBridge(): Promise<void> {
  // Idempotent: if already running, skip re-starting.
  if (abort) return;
  abort = new AbortController();
  const url = `${CLAUDEMON_API_URL}/statusline/stream`;
  console.log(`[claudemon-statusline] subscribed to ${url}`);
  await consumeSseStream(url, {
    signal: abort.signal,
    backoffInitialMs: 200,
    backoffMaxMs: 5000,
    joinWith: '\n',
    onFrame(dataString) {
      let update: any;
      try {
        update = JSON.parse(dataString);
      } catch (err) {
        console.warn('[claudemon-statusline] malformed JSON frame, skipping:', err);
        return;
      }
      try {
        const sl = update.status_line ?? {};
        // Map the wire (snake_case) shape to the renderer's camelCase type.
        claudeSessionStore.applyStatusLine(update.session_id, {
          modelDisplay: sl.model_display,
          contextUsedPct: sl.context_used_pct,
          contextWindowSize: sl.context_window_size,
          totalInputTokens: sl.total_input_tokens,
          totalOutputTokens: sl.total_output_tokens,
          costUSD: sl.cost_usd,
          fiveHourPct: sl.five_hour_pct,
          fiveHourResetsAt: sl.five_hour_resets_at,
          sevenDayPct: sl.seven_day_pct,
          sevenDayResetsAt: sl.seven_day_resets_at,
          monthlyPct: sl.monthly_pct,
          monthlyResetsAt: sl.monthly_resets_at,
          rateLimitWarning: sl.rate_limit_warning,
          overageOutOfCredits: sl.overage_out_of_credits,
          receivedAt: sl.received_at,
        });
        raiseRateLimitWarning(sl.rate_limit_warning);
      } catch (err) {
        console.error('[claudemon-statusline] bad frame', err);
      }
    },
    onError(err) {
      console.warn('[claudemon-statusline] stream error, retrying:', err);
    },
  });
}

export function stopClaudemonStatusLineBridge(): void {
  if (abort) {
    try {
      abort.abort();
    } catch {}
    abort = null;
  }
}
