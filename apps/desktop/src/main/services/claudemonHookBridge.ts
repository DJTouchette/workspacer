/**
 * Consumes the daemon's `/hooks/stream` SSE feed and forwards each event into
 * `claudeSessionStore.handleHookEvent` — the same code path the old in-process
 * hook server used. Reconnects with exponential backoff if the daemon
 * restarts.
 */

import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';

let abort: AbortController | null = null;

export async function startClaudemonHookBridge(): Promise<void> {
  // Idempotent: if already running, skip re-starting.
  if (abort) return;
  abort = new AbortController();
  const url = `${CLAUDEMON_API_URL}/hooks/stream`;
  console.log(`[claudemon-bridge] subscribed to ${url}`);
  await consumeSseStream(url, {
    signal: abort.signal,
    backoffInitialMs: 200,
    backoffMaxMs: 5000,
    joinWith: '\n',
    onFrame(dataString) {
      let event: unknown;
      try {
        event = JSON.parse(dataString);
      } catch (err) {
        console.warn('[claudemon-bridge] malformed JSON frame, skipping:', err);
        return;
      }
      try {
        // claudemon emits `hook_event_name` as `event`. Translate back so
        // claudeSessionStore (which reads `hook_event_name ?? type`) is happy.
        // The Rust serializer also flattens payload into the same object, so
        // tool_name / tool_input / etc. are top-level — matching the schema
        // Claude Code itself POSTs.
        const normalized = { ...(event as Record<string, unknown>), hook_event_name: (event as any).event };
        claudeSessionStore.handleHookEvent(normalized);
      } catch (err) {
        console.error('[claudemon-bridge] bad frame', err);
      }
    },
    onError(err) {
      console.warn(`[claudemon-bridge] stream error, retrying:`, err);
    },
  });
}

export function stopClaudemonHookBridge(): void {
  if (abort) {
    try { abort.abort(); } catch {}
    abort = null;
  }
}
