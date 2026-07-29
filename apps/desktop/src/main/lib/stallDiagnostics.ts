/**
 * Stall diagnostics for the main process.
 *
 * Freezes that take the WHOLE app down are almost always one of two things:
 * the main process's event loop blocking (no IPC is serviced, so every pane
 * stops at once), or the renderer's main thread blocking. These two monitors
 * cover the main-process half; `lib/longTaskMonitor.ts` in the renderer covers
 * the other. Between them, a reported freeze names its own culprit instead of
 * being narrowed down by guesswork.
 *
 * Both are cheap enough to leave on permanently: the lag monitor is one timer
 * per second, and the IPC wrapper only formats a message when a handler has
 * already blown past its threshold.
 *
 * Thresholds are env-tunable (WKS_STALL_MS / WKS_SLOW_IPC_MS) so a user chasing
 * a subtle stutter can lower them without a rebuild.
 */
import { ipcMain } from 'electron';

const numFromEnv = (name: string, fallback: number): number => {
  const n = Number(process.env[name]);
  return Number.isFinite(n) && n > 0 ? n : fallback;
};

/** Event-loop lag above this is a user-visible stall, not scheduling noise. */
const STALL_MS = numFromEnv('WKS_STALL_MS', 250);
/** An IPC handler slower than this is a candidate freeze cause. */
const SLOW_IPC_MS = numFromEnv('WKS_SLOW_IPC_MS', 100);
/** How often the lag monitor checks in. */
const TICK_MS = 1_000;
/** Sync stretches shorter than this are scheduling noise, not worth recording. */
const SYNC_FLOOR_MS = 5;
/** How much of a stall the recorded handlers must explain before we blame them. */
const ATTRIBUTION_RATIO = 0.8;
/** Bound on the blocking-history ring — a stall only ever looks back one tick. */
const MAX_BLOCKING_RECORDS = 32;

/**
 * Monotonic. `Date.now()` walks backwards on an NTP step and jumps hours on
 * suspend/resume, either of which would print a fictional multi-hour stall next
 * to the real ones in a log someone attached to a bug report.
 */
const now = (): number => performance.now();

/**
 * IPC handlers that have been entered and not yet returned, keyed by a call id
 * so overlapping handlers finishing out of order can't corrupt the set. Note
 * that by the time anything reads this, every entry is *suspended at an await* —
 * a handler still running synchronously would be blocking this very read.
 */
const active = new Map<number, string>();
let callSeq = 0;

/** Synchronous stretches recently spent inside an IPC handler. */
type BlockingRecord = { channel: string; endedAt: number; syncMs: number };
let recentBlocking: BlockingRecord[] = [];

const recordBlocking = (record: BlockingRecord): void => {
  recentBlocking.push(record);
  if (recentBlocking.length > MAX_BLOCKING_RECORDS) {
    recentBlocking = recentBlocking.slice(-MAX_BLOCKING_RECORDS);
  }
};

/**
 * Explain a stall, given the window it happened in.
 *
 * The subtlety this exists to handle: a handler that blocks the loop
 * synchronously is *never* in `active` when the lag timer finally gets to run.
 * The wrapper is async, so its `finally` resolves in a microtask at the end of
 * the same macrotask — strictly before the timers phase. Reading a "currently
 * executing" variable from a timer that can only run once the blocking is over
 * always reports nothing, which is precisely the case the monitor exists for.
 *
 * So attribution runs off the recorded history of synchronous stretches
 * instead, and only claims a culprit when those stretches actually account for
 * the stall. Handlers merely awaiting are reported separately and hedged: they
 * are suspects only in the sense of being nearby.
 *
 * Exported for tests — the attribution rule is the whole point of this module.
 */
export function attributeStall(lag: number, windowStart: number): string {
  const inWindow = recentBlocking.filter((r) => r.endedAt > windowStart);
  const blockedMs = inWindow.reduce((sum, r) => sum + r.syncMs, 0);

  if (inWindow.length > 0 && blockedMs >= lag * ATTRIBUTION_RATIO) {
    const byChannel = new Map<string, number>();
    for (const r of inWindow) byChannel.set(r.channel, (byChannel.get(r.channel) ?? 0) + r.syncMs);
    const culprits = [...byChannel.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 3)
      .map(([channel, ms]) => `ipc:${channel} (${Math.round(ms)}ms)`)
      .join(', ');
    return ` — blocked synchronously in ${culprits}`;
  }

  const awaiting = [...new Set(active.values())];
  if (awaiting.length > 0) {
    // Deliberately not phrased as a cause: these are suspended at an await, so
    // whatever blocked the loop was almost certainly something else.
    return ` (not IPC; ${awaiting.length} handler(s) awaiting: ${awaiting.slice(0, 3).join(', ')})`;
  }
  return ' (no IPC in flight)';
}

/**
 * Detect main-process event-loop blocking. A timer set for TICK_MS that fires
 * appreciably later means something ran synchronously in between and nothing
 * else — no IPC, no PTY forwarding, no window events — could be serviced.
 *
 * This catches blocking from sources an IPC wrapper can't see: sync fs work in
 * a watcher callback, a big JSON.parse on an SSE frame, a synchronous child
 * process, GC pauses.
 */
export function startEventLoopLagMonitor(): void {
  let last = now();
  const timer = setInterval(() => {
    const tickedAt = now();
    const lag = tickedAt - last - TICK_MS;
    const windowStart = last;
    last = tickedAt;
    if (lag >= STALL_MS) {
      console.warn(`[stall] main process blocked ~${Math.round(lag)}ms${attributeStall(lag, windowStart)}`);
    }
    // Anything older than the window just closed can never be attributed again.
    recentBlocking = recentBlocking.filter((r) => r.endedAt > tickedAt - TICK_MS);
  }, TICK_MS);
  // Don't hold the process open on quit.
  timer.unref?.();
}

/**
 * Time every `ipcMain.handle` callback and warn on the slow ones.
 *
 * Wraps the method itself rather than each of the ~110 call sites, so handlers
 * registered later (and by other modules) are covered automatically and nobody
 * has to remember to opt in. Must be called BEFORE the handlers register.
 */
export function instrumentIpcHandlers(): void {
  const original = ipcMain.handle.bind(ipcMain);
  ipcMain.handle = ((channel: string, listener: (...args: any[]) => any) => {
    return original(channel, async (...args: any[]) => {
      const id = ++callSeq;
      active.set(id, channel);
      const started = now();
      let syncEndedAt = started;
      try {
        let result: unknown;
        try {
          result = listener(...args);
        } finally {
          // Measured around the *call*, not the await: this is the stretch that
          // actually held the event loop. A synchronous handler is entirely
          // this; an async one contributes whatever it did before its first
          // suspension point. A sync throw is covered too.
          syncEndedAt = now();
        }
        // Await so async handlers are measured end-to-end, not just to their
        // first suspension point. A handler that's slow only because it awaits
        // the network is worth seeing too — it just won't correlate with a lag
        // report, which is exactly how we tell the two apart.
        return await result;
      } finally {
        active.delete(id);
        const syncMs = syncEndedAt - started;
        if (syncMs >= SYNC_FLOOR_MS) recordBlocking({ channel, endedAt: syncEndedAt, syncMs });
        const elapsed = now() - started;
        if (elapsed >= SLOW_IPC_MS) {
          console.warn(`[slow-ipc] ${channel} took ${Math.round(elapsed)}ms`);
        }
      }
    });
  }) as typeof ipcMain.handle;
}

/** Reset module state between tests. Not used in production. */
export function __resetStallDiagnostics(): void {
  active.clear();
  recentBlocking = [];
  callSeq = 0;
}
