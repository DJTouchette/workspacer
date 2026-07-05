/**
 * Consumes the daemon's `/events` SSE feed (session-state updates) and folds a
 * *managed* session's mode into the store's `ambientState`.
 *
 * Claude sessions get their working/idle/waiting state from hooks
 * (`claudemonHookBridge`). Managed adapters (Codex / OpenCode / Pi) fire no
 * hooks — claudemon instead broadcasts their mode here as `session.update`
 * frames (event `"Managed"`). Without this bridge a managed session's status is
 * stuck on the `'idle'` default no matter what the agent is doing.
 */

import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';

let abort: AbortController | null = null;

export async function startClaudemonEventBridge(): Promise<void> {
  // Idempotent: if already running, skip re-starting.
  if (abort) return;
  abort = new AbortController();
  const url = `${CLAUDEMON_API_URL}/events`;
  console.log(`[claudemon-events] subscribed to ${url}`);
  await consumeSseStream(url, {
    signal: abort.signal,
    backoffInitialMs: 200,
    backoffMaxMs: 5000,
    joinWith: '\n',
    onFrame(dataString) {
      let update: { session_id?: string; event?: string; state?: { mode?: string } };
      try {
        update = JSON.parse(dataString);
      } catch (err) {
        console.warn('[claudemon-events] malformed JSON frame, skipping:', err);
        return;
      }
      // Only managed sessions emit mode changes via `set_managed_mode` (event
      // "Managed"). Ignore Spawn / SessionEnd and any Claude-PTY updates — their
      // ambientState is hook-driven and must not be clobbered here.
      if (update?.event !== 'Managed') return;
      const mode = update.state?.mode;
      if (update.session_id && typeof mode === 'string') {
        claudeSessionStore.applyManagedMode(update.session_id, mode);
      }
    },
    onError(err) {
      console.warn(`[claudemon-events] stream error, retrying:`, err);
    },
  });
}

export function stopClaudemonEventBridge(): void {
  if (abort) {
    try {
      abort.abort();
    } catch {}
    abort = null;
  }
}
