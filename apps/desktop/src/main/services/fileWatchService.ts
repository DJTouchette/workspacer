/**
 * Single-file watch backend for the editor pane (external-change detection).
 *
 * Like fileService, this is the app's own fs layer — exposed two ways so both
 * clients learn when a file changed underneath the buffer: `file:watch`/
 * `file:changed` IPC for the desktop renderer, and `fs.watch`/`fs.changed`
 * hub events for the web/phone client. Execution lives here in main; the hub
 * only routes.
 *
 * fs.watch is intentionally low-level: it fires 1-3 times per write and offers
 * no payload beyond an event kind. We coalesce duplicates within a short window
 * and refcount one watcher per path, then hand a single { path, eventType }
 * event to a caller-installed sink. The sink is set once at startup by main and
 * fans the event out to *both* the IPC and hub layers.
 */
import * as fs from 'fs';
import * as path from 'path';

/** fs.watch can fire several times for one save — collapse bursts this tight. */
const DEBOUNCE_MS = 50;

export interface FileChangeEvent {
  path: string;
  eventType: 'change' | 'rename';
}

type EmitSink = (event: FileChangeEvent) => void;

interface WatchEntry {
  watcher: fs.FSWatcher;
  refcount: number;
  leases: Map<string, number>;
  /** Pending debounce timer + the kind of the last event seen in the window. */
  timer: NodeJS.Timeout | null;
  lastEventType: FileChangeEvent['eventType'];
}

/** path (resolved) → shared watcher. */
const watches = new Map<string, WatchEntry>();
let leaseSweep: ReturnType<typeof setInterval> | undefined;
function sweepLeases(): void {
  let any = false;
  for (const [file, entry] of watches) {
    for (const [id, expiry] of entry.leases) if (Date.now() > expiry) entry.leases.delete(id);
    if (entry.refcount === 0 && entry.leases.size === 0) disposeWatch(file, entry);
    else if (entry.leases.size) any = true;
  }
  if (!any && leaseSweep) { clearInterval(leaseSweep); leaseSweep = undefined; }
}
function lease(entry: WatchEntry, id?: string): void {
  if (!id) { entry.refcount++; return; }
  if (id.length > 128 || (entry.leases.size >= 128 && !entry.leases.has(id))) throw new Error('Too many file watchers or invalid watchId');
  entry.leases.set(id, Date.now() + 180_000);
  if (!leaseSweep) { leaseSweep = setInterval(sweepLeases, 30_000); leaseSweep.unref?.(); }
}

/**
 * Where coalesced events go. Set once by main at startup; defaults to a no-op so
 * the service is safe to call before wiring (and in tests).
 */
let emit: EmitSink = () => {};

/**
 * Install the global event sink. Called once from main with a fan-out that
 * pushes to the renderer (webContents.send) *and* publishes onto the hub bus.
 */
export function setEmitSink(sink: EmitSink): void {
  emit = sink;
}

/**
 * Start (or join) a watch on `filePath`. Multiple calls for the same path share
 * one fs.watch and bump a refcount, so unwatching is symmetric. Events are
 * debounced per path and delivered through the registered sink — `onEvent` is
 * accepted for symmetry/testing but the production path uses the global sink so
 * one watcher serves both transports.
 */
export function startWatch(filePath: string, onEvent?: EmitSink, watchId?: string): void {
  if (watchId !== undefined && (typeof watchId !== 'string' || watchId.length > 128)) throw new Error('Invalid watchId');
  const resolved = path.resolve(filePath);
  const existing = watches.get(resolved);
  if (existing) {
    lease(existing, watchId);
    return;
  }

  let watcher: fs.FSWatcher;
  try {
    let directory = false;
    try { directory = fs.statSync(resolved).isDirectory(); }
    catch (err) { if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err; }
    watcher = fs.watch(directory ? resolved : path.dirname(resolved), (eventType, filename) => {
      if (!directory && filename && filename.toString() !== path.basename(resolved)) return;
      // fs.watch's callback must never throw — a thrown error here crashes the
      // watcher with no recovery. Guard everything inside.
      try {
        const entry = watches.get(resolved);
        if (!entry) return;
        // Node types eventType as string; narrow to our union (anything that
        // isn't 'rename' is treated as a content change).
        entry.lastEventType = eventType === 'rename' ? 'rename' : 'change';
        if (entry.timer) return; // already coalescing this burst
        entry.timer = setTimeout(() => {
          entry.timer = null;
          const ev: FileChangeEvent = { path: resolved, eventType: entry.lastEventType };
          try {
            emit(ev);
          } catch {
            /* sink must not break the watcher */
          }
          try {
            onEvent?.(ev);
          } catch {
            /* ditto */
          }
        }, DEBOUNCE_MS);
      } catch (err) {
        // Keep the watcher alive, but make the failure visible — swallowing it
        // silently hides genuine bugs in the debounce/emit path.
        console.error(`[fileWatch] event handler failed for ${resolved}:`, err);
      }
    });
  } catch (err) {
    // ENOENT (file deleted/never existed) or EMFILE etc. — don't register a
    // broken entry; the caller's watch simply yields no events.
    console.warn(`[fileWatch] cannot watch ${resolved}: ${(err as Error).message}`);
    return;
  }

  // A watcher whose file is later removed emits an 'error' (often ENOENT). Keep
  // it from becoming an unhandled exception; the rename event already told the
  // renderer the file went away.
  watcher.on('error', (err) => {
    console.warn(`[fileWatch] watcher error for ${resolved}: ${(err as Error).message}`);
  });

  const entry: WatchEntry = { watcher, refcount: 0, leases: new Map(), timer: null, lastEventType: 'change' };
  lease(entry, watchId);
  watches.set(resolved, entry);
}

/**
 * Drop one reference to `filePath`'s watch; the underlying fs.watch is closed
 * only when the last watcher releases it. Safe to call for an unknown path.
 */
export function stopWatch(filePath: string, watchId?: string): void {
  const resolved = path.resolve(filePath);
  const entry = watches.get(resolved);
  if (!entry) return;
  if (watchId) entry.leases.delete(watchId);
  else entry.refcount = Math.max(0, entry.refcount - 1);
  if (entry.refcount > 0 || entry.leases.size > 0) return;
  disposeWatch(resolved, entry);
}

function disposeWatch(resolved: string, entry: WatchEntry): void {
  if (entry.timer) {
    clearTimeout(entry.timer);
    entry.timer = null;
  }
  try {
    entry.watcher.close();
  } catch {
    /* already closed */
  }
  watches.delete(resolved);
}
