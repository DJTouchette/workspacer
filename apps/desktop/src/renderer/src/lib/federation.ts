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
