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
import { readSelectionSlice } from '../shared/canonicalSelection';

let abort: AbortController | null = null;

type StateUpdate = {
  session_id?: string;
  event?: string;
  state?: {
    mode?: string;
    provider?: string;
    transport?: string;
    updated_at?: string;
    pending?: ManagedPendingWire | null;
    execution_engine?: import('../shared/executionEngine').ExecutionEngineMetadata;
    background_tasks?: number;
    subagents?: unknown[];
    /** claudemon's canonical selection slice, in its own spelling. Both
     *  optional — an older daemon sends neither, which is why the reader
     *  below reports absence rather than a default. */
    requested_selection?: { model?: string; context_window?: number | null } | null;
    resolved_context_window?: number | null;
  };
};

// RFC3339 timestamps carry nanoseconds in the daemon. Date.parse alone loses
// ordering between two transitions in the same millisecond.
function compareStateTime(a?: string, b?: string): number | null {
  if (!a || !b) return null;
  const am = Date.parse(a),
    bm = Date.parse(b);
  if (!Number.isFinite(am) || !Number.isFinite(bm)) return null;
  if (am !== bm) return am - bm;
  const submillis = (value: string) =>
    Number((value.match(/\.(\d+)/)?.[1] ?? '').padEnd(9, '0').slice(3, 9));
  return submillis(a) - submillis(b);
}

function applyStateUpdate(update: StateUpdate): void {
  if (update.session_id && update.state?.execution_engine) {
    if (
      claudeSessionStore.applyExecutionEngine(update.session_id, update.state.execution_engine) ===
      false
    )
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
      // Ambient background work (shells, subagents) — badge, never "Working".
      backgroundTasks: update.state?.background_tasks,
      subagents: update.state?.subagents,
      // The daemon OWNS the canonical selection slice; this is the only
      // channel that carries it live. Mapped snake_case → camelCase here
      // and merged presence-aware in the store, so a frame that omits
      // either field cannot erase what the row already knows.
      selection: readSelectionSlice(update.state),
      ...(update.state?.execution_engine ? { executionEngine: update.state.execution_engine } : {}),
    });
  }
}

export async function startClaudemonEventBridge(): Promise<void> {
  if (abort) return;
  const controller = new AbortController();
  abort = controller;
  const { signal } = controller;
  let connection = 0;
  let syncing = false;
  let syncAgain = false;
  let changedDuringSync: Map<string, StateUpdate> | undefined;
  const reconciledAt = new Map<string, string>();
  function rememberReconciliation(id: string, at?: string): void {
    if (!at) return;
    reconciledAt.delete(id);
    reconciledAt.set(id, at);
    // Retain terminal timestamps briefly too: already-buffered old frames
    // must not resurrect a session just reconciled as ended. Bound retention
    // independently of daemon uptime and release it on every new connection.
    if (reconciledAt.size > 4096) reconciledAt.delete(reconciledAt.keys().next().value!);
  }
  let retry: ReturnType<typeof setTimeout> | undefined;
  signal.addEventListener(
    'abort',
    () => {
      if (retry) clearTimeout(retry);
    },
    { once: true },
  );

  async function reconcile(): Promise<void> {
    if (signal.aborted) return;
    if (syncing) {
      syncAgain = true;
      return;
    }
    if (retry) {
      clearTimeout(retry);
      retry = undefined;
    }
    syncing = true;
    const generation = connection;
    const changed = new Map<string, StateUpdate>();
    changedDuringSync = changed;
    try {
      // Include even empty/archived stopped rows: an exit may have been the
      // very event dropped. Only reconcile sessions this desktop already owns.
      const response = await fetch(
        `${CLAUDEMON_API_URL}/sessions?include_archived=true&include_empty=true&state_only=true`,
        {
          signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
        },
      );
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const body: unknown = await response.json();
      if (!Array.isArray(body)) throw new Error('Invalid session reconciliation response');
      const states = body as Array<NonNullable<StateUpdate['state']> & { session_id?: string }>;
      if (signal.aborted || generation !== connection) return;
      for (const state of states) {
        const id = state.session_id;
        if (!id) continue;
        // Frames queued before the lag marker can still arrive during this
        // fetch. Compare daemon time before deciding that a frame is newer.
        if (changed.has(id)) {
          const order = compareStateTime(state.updated_at, changed.get(id)?.state?.updated_at);
          if (order === null || order <= 0) continue;
        }
        const current = claudeSessionStore.getSnapshot(id);
        if (!current || current.hub || current.status === 'ended') continue;
        // Claude PTY activity remains hook-owned. Managed providers and Claude
        // stream sessions use the authoritative mode/pending slot here.
        const managed =
          state.transport === 'stream' ||
          (typeof state.provider === 'string' && state.provider !== 'claude');
        if (state.mode === 'stopped') {
          applyStateUpdate({ event: 'SessionEnd', session_id: id, state });
          rememberReconciliation(id, state.updated_at);
        } else if (managed) {
          applyStateUpdate({ event: 'Managed', session_id: id, state });
          rememberReconciliation(id, state.updated_at);
        }
      }
    } catch (err) {
      if (!signal.aborted) {
        console.warn('[claudemon-events] state reconciliation failed, retrying:', err);
        retry = setTimeout(() => {
          retry = undefined;
          void reconcile();
        }, 1000);
        retry.unref?.();
      }
    } finally {
      syncing = false;
      changedDuringSync = undefined;
      if (syncAgain && !signal.aborted) {
        syncAgain = false;
        void reconcile();
      }
    }
  }

  const url = `${CLAUDEMON_API_URL}/events`;
  console.log(`[claudemon-events] subscribed to ${url}`);
  await consumeSseStream(url, {
    signal,
    backoffInitialMs: 200,
    backoffMaxMs: 5000,
    joinWith: '\n',
    onConnect() {
      connection++;
      reconciledAt.clear();
      void reconcile();
    },
    onFrame(dataString) {
      let update: StateUpdate;
      try {
        update = JSON.parse(dataString);
      } catch (err) {
        console.warn('[claudemon-events] malformed JSON frame, skipping:', err);
        return;
      }
      if (!update || typeof update !== 'object') return;
      if (update.event === 'Resync') {
        void reconcile();
        return;
      }
      if (update.session_id) {
        const order = compareStateTime(
          update.state?.updated_at,
          reconciledAt.get(update.session_id),
        );
        if (order !== null && order < 0) return;
        if (order !== null) reconciledAt.delete(update.session_id);
        changedDuringSync?.set(update.session_id, update);
      }
      applyStateUpdate(update);
    },
    onError(err) {
      console.warn('[claudemon-events] stream error, retrying:', err);
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
