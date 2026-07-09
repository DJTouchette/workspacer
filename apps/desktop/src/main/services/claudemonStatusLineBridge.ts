/**
 * Consumes the daemon's `/statusline/stream` SSE feed and forwards each tick
 * into `claudeSessionStore.applyStatusLine`. This is a separate channel from
 * `/hooks/stream` because the statusLine command fires very frequently — it is
 * deliberately kept off the hook fanout (and thus off the SQLite persistence
 * path). Reconnects with exponential backoff if the daemon restarts.
 */

import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';

let abort: AbortController | null = null;

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
          receivedAt: sl.received_at,
        });
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
