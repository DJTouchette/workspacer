import type { ClaudeSessionSnapshot, SessionAmbientState } from '../types/claudeSession';
import { compactClaudeSnapshotForBackground } from './compactClaudeSnapshot';
import { wasSessionTerminated } from './terminatedSessions';

export interface PromotedSessionMaps {
  statusBySession: Record<string, SessionAmbientState>;
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
}

/**
 * Build the promoted status/snapshot maps from a full daemon session list.
 *
 * Skips two kinds of session:
 *  - ones the user explicitly terminated this run (tombstoned), and
 *  - ones the daemon already reports as `ended`.
 *
 * Ended sessions are returned by getAllClaudeSessions as resumable Stopped
 * rows, but they never emit another `onClaudeSessionUpdate` tick — so if we
 * promoted one here, the live-update cleanup (which only evicts on a tick with
 * status === 'ended') could never remove it, and it would leak in memory for
 * the app's lifetime, re-accumulating on every reconnect. This mirrors the
 * exclusion already applied by the live-update path and boot reconciliation
 * (useSessionLifecycle's live-session-ids check).
 */
export function promoteSessionSnapshots(sessions: ClaudeSessionSnapshot[]): PromotedSessionMaps {
  const statusBySession: Record<string, SessionAmbientState> = {};
  const snapshotBySession: Record<string, ClaudeSessionSnapshot> = {};
  for (const s of sessions) {
    if (s.status === 'ended') continue;
    if (wasSessionTerminated(s.sessionId)) continue;
    statusBySession[s.sessionId] = s.ambientState;
    snapshotBySession[s.sessionId] = compactClaudeSnapshotForBackground(s);
  }
  return { statusBySession, snapshotBySession };
}

/**
 * Whether a live `onClaudeSessionUpdate` tick should evict its session from the
 * promoted maps instead of refreshing it.
 *
 * The mirror of the exclusions in [`promoteSessionSnapshots`]: an `ended`
 * session never ticks again, so its (full-transcript) snapshot would stay
 * pinned forever. An explicitly terminated one is evicted even before the
 * daemon reports `ended`, because its teardown ticks — final hooks, a last
 * status line — would otherwise re-promote the snapshot and let auto-adopt
 * resurrect the card the user just closed.
 */
export function shouldEvictSession(
  sessionId: string,
  status: ClaudeSessionSnapshot['status'] | undefined,
): boolean {
  return status === 'ended' || wasSessionTerminated(sessionId);
}

/**
 * Drop one session's entry from a promoted map.
 *
 * Returns the *same* object when the key isn't there. That identity is
 * load-bearing: these maps are React state, and returning a fresh object for a
 * session we never held would re-render every consumer on each teardown tick
 * of an unrelated session.
 */
export function omitSession<T>(record: Record<string, T>, sessionId: string): Record<string, T> {
  if (!(sessionId in record)) return record;
  const { [sessionId]: _drop, ...rest } = record;
  return rest;
}
