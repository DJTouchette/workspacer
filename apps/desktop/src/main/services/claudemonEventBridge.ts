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

import { claudeSessionStore, type ManagedPendingWire } from './claudeSessionStore';
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
      let update: {
        session_id?: string;
        event?: string;
        state?: {
          mode?: string;
          provider?: string;
          transport?: string;
          pending?: ManagedPendingWire | null;
        };
      };
      try {
        update = JSON.parse(dataString);
      } catch (err) {
        console.warn('[claudemon-events] malformed JSON frame, skipping:', err);
        return;
      }
      // A managed session's process exiting comes through as a SessionEnd frame
      // (claudemon's `deregister_managed` sets mode=Stopped and broadcasts event
      // "SessionEnd"). Managed backends fire no Claude hooks, so this is the ONLY
      // signal that ends them — route it into the store's ended pipeline
      // (status -> 'ended', history write, per-session eviction) via a synthetic
      // SessionEnd hook. Without this the card stays 'live/idle' forever and the
      // session's maps leak for the process lifetime.
      if (update?.event === 'SessionEnd' && update.session_id) {
        claudeSessionStore.handleHookEvent({
          hook_event_name: 'SessionEnd',
          session_id: update.session_id,
        });
        return;
      }
      // Only managed sessions emit mode changes via `set_managed_mode` (event
      // "Managed"). Ignore Spawn and any Claude-PTY updates — their
      // ambientState is hook-driven and must not be clobbered here.
      if (update?.event !== 'Managed') return;
      const mode = update.state?.mode;
      if (update.session_id && typeof mode === 'string') {
        // Forward the daemon's backend identity too: a session the desktop
        // didn't spawn this run (adopted, or restored after an app restart)
        // has no spawn metadata, and for a stream-transport Claude session
        // the transport gates the whole pane (no Term view, /answer path,
        // hooks-enrichment-only guard).
        claudeSessionStore.applyManagedMode(update.session_id, mode, {
          provider: update.state?.provider,
          transport: update.state?.transport,
          // Managed adapters fire no PermissionRequest/AskUserQuestion hooks —
          // the daemon's `pending` slot is their only approval/question payload.
          pending: update.state?.pending ?? null,
        });
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
