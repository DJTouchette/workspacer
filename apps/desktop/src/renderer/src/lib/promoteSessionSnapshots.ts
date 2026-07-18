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
 * exclusion already applied by the live-update path and reconcileWithDaemon.
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
