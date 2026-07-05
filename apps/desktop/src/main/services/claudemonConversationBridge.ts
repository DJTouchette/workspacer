/**
 * Consumes the daemon's `/conversation/stream` SSE feed — structured
 * conversation items parsed from each session's JSONL transcript by
 * claudemon's tailer — and folds each delta into `claudeSessionStore`.
 * This replaces the old in-process transcript re-parsing: the daemon owns
 * the JSONL; this process only renders. Reconnects with exponential
 * backoff if the daemon restarts (sequence gaps are then detected by the
 * store, which resyncs from the snapshot endpoint).
 */

import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';

let abort: AbortController | null = null;

export async function startClaudemonConversationBridge(): Promise<void> {
  // Idempotent: if already running, skip re-starting.
  if (abort) return;
  abort = new AbortController();
  const url = `${CLAUDEMON_API_URL}/conversation/stream`;
  console.log(`[claudemon-conversation] subscribed to ${url}`);
  await consumeSseStream(url, {
    signal: abort.signal,
    backoffInitialMs: 200,
    backoffMaxMs: 5000,
    joinWith: '\n',
    onFrame(dataString) {
      let delta: unknown;
      try {
        delta = JSON.parse(dataString);
      } catch (err) {
        console.warn('[claudemon-conversation] malformed JSON frame, skipping:', err);
        return;
      }
      try {
        claudeSessionStore.applyConversationDelta(
          delta as Parameters<typeof claudeSessionStore.applyConversationDelta>[0],
        );
      } catch (err) {
        console.error('[claudemon-conversation] bad frame', err);
      }
    },
    onError(err) {
      console.warn('[claudemon-conversation] stream error, retrying:', err);
    },
  });
}

export function stopClaudemonConversationBridge(): void {
  if (abort) {
    try {
      abort.abort();
    } catch {}
    abort = null;
  }
}
