/**
 * Bridges a session's live PTY output to remote bus clients so the remote web
 * client can mirror the real Claude Code terminal pane.
 *
 * claudemon exposes the raw terminal bytes at `GET /sessions/:id/stream` (SSE,
 * base64 chunks, first event replays the ring buffer). The remote can't reach
 * claudemon directly, so for each session a remote is *watching* we run one SSE
 * consumer here and republish every chunk onto the hub bus as
 * `pty.bytes.<sessionId>` events. The remote subscribes to that topic and feeds
 * the bytes into xterm.js.
 *
 * Streaming is lease-gated: a remote calls `attachTerminal` when it opens the
 * terminal view (which (re)starts the stream so the buffer replay re-primes the
 * viewer) and refreshes the lease with `keepaliveTerminal` on a timer. When the
 * lease lapses (viewer left, phone slept, network dropped) the stream stops, so
 * we never stream a session nobody is looking at.
 */

import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { publishToHub } from './hubClient';
import { consumeSseStream } from '../lib/sseConsumer';

const LEASE_MS = 20_000;
const SWEEP_MS = 5_000;
const BACKOFF_INITIAL_MS = 500;
const BACKOFF_MAX_MS = 5_000;
/** Coalesce PTY chunks into one bus event per frame (~60fps) so bursty output
 *  (build logs, `npm test`) doesn't flood a phone with thousands of tiny WS
 *  messages. Flush early if the pending buffer gets large to bound latency. */
const FLUSH_MS = 16;
const FLUSH_BYTES = 64 * 1024;

interface Forwarder {
  sessionId: string;
  abort: AbortController;
  deadline: number;
  stopped: boolean;
  /** Decoded chunks awaiting a coalesced flush. */
  pending: Buffer[];
  pendingBytes: number;
  flushTimer: ReturnType<typeof setTimeout> | null;
}

const forwarders = new Map<string, Forwarder>();
let sweeper: ReturnType<typeof setInterval> | null = null;

function ensureSweeper(): void {
  if (sweeper) return;
  sweeper = setInterval(() => {
    const now = Date.now();
    for (const f of forwarders.values()) {
      if (now > f.deadline) stopTerminal(f.sessionId);
    }
    if (forwarders.size === 0 && sweeper) {
      clearInterval(sweeper);
      sweeper = null;
    }
  }, SWEEP_MS);
}

/**
 * Start (or restart) forwarding a session's PTY to the bus and take a fresh
 * lease. Restarting is intentional: the new SSE connection replays the ring
 * buffer, which re-primes whichever viewer just attached.
 */
export function attachTerminal(sessionId: string): void {
  stopTerminal(sessionId);
  const f: Forwarder = {
    sessionId,
    abort: new AbortController(),
    deadline: Date.now() + LEASE_MS,
    stopped: false,
    pending: [],
    pendingBytes: 0,
    flushTimer: null,
  };
  forwarders.set(sessionId, f);
  ensureSweeper();
  runStream(f).catch((err) =>
    console.error(`[terminalShare] stream ${sessionId} ended:`, err),
  );
}

/**
 * Refresh the lease. Returns false if no forwarder is active (lease lapsed) so
 * the caller knows to re-attach (and re-prime) rather than assume it's live.
 */
export function keepaliveTerminal(sessionId: string): boolean {
  const f = forwarders.get(sessionId);
  if (!f) return false;
  f.deadline = Date.now() + LEASE_MS;
  return true;
}

/** Stop forwarding a session's PTY (viewer left, or lease lapsed). */
export function stopTerminal(sessionId: string): void {
  const f = forwarders.get(sessionId);
  if (!f) return;
  f.stopped = true;
  if (f.flushTimer) { clearTimeout(f.flushTimer); f.flushTimer = null; }
  try { f.abort.abort(); } catch { /* noop */ }
  forwarders.delete(sessionId);
}

/** Concatenate the buffered chunks and publish them as a single base64 event.
 *  (We can't just concat the base64 strings — only byte concatenation is
 *  correct unless every chunk is a multiple of 3 bytes.) */
function flush(f: Forwarder): void {
  if (f.flushTimer) { clearTimeout(f.flushTimer); f.flushTimer = null; }
  if (f.stopped || f.pending.length === 0) return;
  const combined = Buffer.concat(f.pending, f.pendingBytes);
  f.pending = [];
  f.pendingBytes = 0;
  publishToHub({ type: `pty.bytes.${f.sessionId}`, data: combined.toString('base64') });
}

/** Tear everything down (app shutdown). */
export function stopAllTerminals(): void {
  for (const id of [...forwarders.keys()]) stopTerminal(id);
  if (sweeper) {
    clearInterval(sweeper);
    sweeper = null;
  }
}

async function runStream(f: Forwarder): Promise<void> {
  await consumeSseStream(
    `${CLAUDEMON_API_URL}/sessions/${f.sessionId}/stream`,
    {
      signal: f.abort.signal,
      backoffInitialMs: BACKOFF_INITIAL_MS,
      backoffMaxMs: BACKOFF_MAX_MS,
      joinWith: '',
      onFrame(b64) {
        // Buffer decoded bytes and coalesce; flush on the next frame tick, or
        // immediately once enough has piled up (keeps latency bounded).
        const chunk = Buffer.from(b64, 'base64');
        f.pending.push(chunk);
        f.pendingBytes += chunk.length;
        if (f.pendingBytes >= FLUSH_BYTES) flush(f);
        else if (!f.flushTimer) f.flushTimer = setTimeout(() => flush(f), FLUSH_MS);
      },
      onError(err) {
        console.warn(`[terminalShare] stream ${f.sessionId} error, retrying:`, err);
      },
    },
  );
}
