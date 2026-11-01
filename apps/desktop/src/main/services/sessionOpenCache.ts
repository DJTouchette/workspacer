/**
 * Main's copy of the hub-owned shared open set (`sessionOpen`, see
 * docs/session-roster.md): which sessions every client keeps listed, and reads
 * as Paused once they stop.
 *
 * The hub owns and maintains the document; main only caches it so the
 * `sessions.snapshots` provider and the status-summary gate can apply the
 * roster rule without a round trip per call. The cache converges from:
 *   - `sessionOpen.changed` (the whole document, version-fenced), and
 *   - a `sessionOpen.get` on first use and after every hub (re)connect,
 *     because changes made while main was disconnected have no replay.
 *
 * An unreachable hub, or one too old to serve `sessionOpen`, reads as an empty
 * set: the roster then falls back to the 24h and layout rules.
 */
import { callHub, subscribeHubConnected, subscribeHubEvents, type HubEvent } from './hubClient';

/** `{version, open: {id: sinceMs}, ended: {id: endedAtMs}}`. Paused = stopped
 *  ∧ open ∧ ¬ended; stopped ∧ open ∧ ended = Ended (still on the sidebar). */
export interface SessionOpenDocument {
  version: number;
  open: Record<string, number>;
  ended: Record<string, number>;
}

let document: SessionOpenDocument | null = null;
let started = false;
let inflight: Promise<void> | null = null;
/** Bumped on reconnect so a read started before it cannot win afterwards. */
let epoch = 0;

/** `id → epoch-ms` entries of a map field; null if the field is not a map. */
function idTimes(value: unknown): Record<string, number> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const out: Record<string, number> = {};
  for (const [id, at] of Object.entries(value as Record<string, unknown>)) {
    if (id && typeof at === 'number') out[id] = at;
  }
  return out;
}

function parse(value: unknown): SessionOpenDocument | null {
  const doc = value as { version?: unknown; open?: unknown; ended?: unknown } | null;
  if (!doc || typeof doc !== 'object' || typeof doc.version !== 'number') return null;
  const open = idTimes(doc.open);
  if (!open) return null;
  // `ended` is optional on the wire (an early hub build); absent = none ended.
  return { version: doc.version, open, ended: idTimes(doc.ended) ?? {} };
}

/** Adopt a document unless an equal-or-newer one is held. `force` adopts a
 *  post-reconnect read even if its version is lower (a reset hub). */
function adopt(value: unknown, force = false): void {
  const next = parse(value);
  if (!next) return;
  if (!force && document && document.version >= next.version) return;
  document = next;
}

function refresh(force: boolean): Promise<void> {
  const mine = epoch;
  let read: Promise<void> | undefined = undefined;
  read = (async () => {
    // Yield first so `finally` never runs before `read` is assigned, even if
    // callHub throws synchronously.
    await Promise.resolve();
    try {
      const value = await callHub('sessionOpen.get', {});
      if (mine === epoch) adopt(value, force);
    } catch {
      // Hub down, or too old for sessionOpen: keep what we have.
    } finally {
      if (inflight === read) inflight = null;
    }
  })();
  inflight = read;
  return read;
}

function start(): void {
  if (started) return;
  started = true;
  try {
    subscribeHubEvents((ev: HubEvent) => {
      if (ev.type === 'sessionOpen.changed' && !ev.hub) adopt(ev.data);
    });
    subscribeHubConnected(() => {
      epoch += 1;
      void refresh(true);
    });
  } catch {
    // No hub client in this process (tests, headless tools).
  }
}

/** How long a list call waits for the first read before serving without it. */
const FIRST_READ_BUDGET_MS = 2000;

/** The shared open set's ids. Waits (briefly) for the first read; never throws. */
export async function openSessionIds(): Promise<Set<string>> {
  start();
  if (!document) {
    let timer: NodeJS.Timeout | undefined;
    await Promise.race([
      inflight ?? refresh(false),
      new Promise<void>((resolve) => {
        timer = setTimeout(resolve, FIRST_READ_BUDGET_MS);
      }),
    ]);
    clearTimeout(timer);
  }
  return new Set(Object.keys(document?.open ?? {}));
}

/** The cached document, if one has been read. */
export function sessionOpenDocument(): SessionOpenDocument | null {
  return document;
}

/** Test seam: forget everything. */
export function resetSessionOpenCacheForTests(): void {
  document = null;
  started = false;
  inflight = null;
  epoch = 0;
}
