import type { PaneType } from '../types/pane';
import type { PaneMenuEntry } from './paneMenu';

/**
 * Hub federation (renderer side). A peer hub's sessions arrive through the
 * same snapshot/update flow as local ones, tagged with `hub` (the peer's
 * name) and — when that peer's link drops — `hubOffline` (tombstone state).
 * Everything here is the pure logic the UI shares for those sessions:
 * which local, cwd-bound surfaces to withhold, and how to word the
 * offline/disabled states.
 */

export interface FederationPeer {
  name: string;
  connected: boolean;
  /** Epoch ms of the last successful link activity, when known. */
  lastSeen?: number;
}

/** List the configured peer hubs, or [] when the bridge (or the feature)
 *  isn't there (older preloads / test mocks call through `?.`). Never throws
 *  — callers gate UI on a non-empty result. */
export async function fetchFederationPeers(): Promise<FederationPeer[]> {
  try {
    const peers = await window.electronAPI.federationPeers?.();
    return Array.isArray(peers) ? peers : [];
  } catch {
    return [];
  }
}

/** Built-in panes that are meaningless for a remote agent: they bind to a
 *  local cwd (shell, git worktree, files) that lives on the peer machine.
 *  Chat, browser and inspector stay — those ride the bus. */
export const REMOTE_UNAVAILABLE_PANES: ReadonlySet<PaneType> = new Set<PaneType>([
  'terminal',
  'review',
  'editor',
]);

/** Drop the cwd-bound built-ins from a pane-creation menu when the target
 *  agent lives on another hub. `remoteHub` undefined/'' = local, no-op. */
export function filterPaneMenuForRemote(
  entries: PaneMenuEntry[],
  remoteHub: string | undefined,
): PaneMenuEntry[] {
  if (!remoteHub) return entries;
  return entries.filter((e) => !(e.kind === 'builtin' && REMOTE_UNAVAILABLE_PANES.has(e.type)));
}

/** Tooltip for a control that would act on the local machine only. */
export function remoteDisabledTitle(hub: string): string {
  return `on ${hub} — not available for remote agents`;
}

// ── Remote conversation sync ────────────────────────────────────────────────

/** The snapshot slice the remote-conversation sync reads. */
export interface RemoteConversationTarget {
  sessionId: string;
  hub?: string;
  hubOffline?: boolean;
}

/** `window.electronAPI.federationConversation` (or the web backend twin). */
export type RemoteConversationFetch = (
  sessionId: string,
  sinceSeq?: number,
) => Promise<{ seq: number; items: unknown[] } | null>;

/**
 * The poke-side guard for a remote agent pane's conversation sync.
 *
 * A remote session's snapshot carries only the peer's compacted conversation
 * window; the full transcript lives behind `federationConversation`, which
 * makes main fetch `hub:<peer>/sessions.conversation`, fold it into the
 * session store, and push it back through the normal snapshot flow. The pane
 * pokes this on mount/visibility and on activity bumps; this factory keeps the
 * poking honest (the TUI's begin_resync-style guard):
 *
 *   - local / tombstoned (hubOffline) targets are ignored — a down link must
 *     not be polled; the reconnect reseed flips hubOffline off and the next
 *     poke resumes;
 *   - one fetch in flight at a time; pokes that land meanwhile coalesce into
 *     a single trailing fetch;
 *   - the last seen `seq` is passed as `sinceSeq`, so a poke when nothing
 *     changed costs one empty incremental response, not a transcript.
 */
export function createRemoteConversationSync(fetchConversation: RemoteConversationFetch): {
  poke(target: RemoteConversationTarget | null | undefined): void;
} {
  let tracked: { id: string; seq: number } | null = null;
  let inflight = false;
  let queued: RemoteConversationTarget | null = null;

  const poke = (target: RemoteConversationTarget | null | undefined): void => {
    if (!target?.sessionId || !target.hub || target.hubOffline) return;
    if (inflight) {
      queued = target;
      return;
    }
    inflight = true;
    const since =
      tracked && tracked.id === target.sessionId && tracked.seq > 0 ? tracked.seq : undefined;
    let call: Promise<{ seq: number; items: unknown[] } | null>;
    try {
      call = Promise.resolve(fetchConversation(target.sessionId, since));
    } catch {
      call = Promise.resolve(null); // a sync throw counts as a failed fetch
    }
    call
      .then((res) => {
        if (res && typeof res.seq === 'number') tracked = { id: target.sessionId, seq: res.seq };
      })
      .catch(() => {
        // Link errors surface as tombstones through the snapshot flow; the
        // poke stream just keeps quiet here.
      })
      .then(() => {
        inflight = false;
        const next = queued;
        queued = null;
        if (next) poke(next);
      });
  };

  return { poke };
}

/** Compact "3h" / "5m" style duration for last-seen copy. */
function relDur(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

/** Tombstone state text for a session whose peer hub link is down. */
export function hubOfflineLabel(lastActivity: number | undefined, now: number): string {
  if (!lastActivity) return 'hub offline';
  return `hub offline — last seen ${relDur(now - lastActivity)} ago`;
}
