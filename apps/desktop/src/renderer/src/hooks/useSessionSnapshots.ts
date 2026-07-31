import { useCallback, useEffect, useRef, useState } from 'react';
import type { ClaudeSessionSnapshot, SessionAmbientState } from '../types/claudeSession';
import { compactClaudeSnapshotForBackground } from '../lib/compactClaudeSnapshot';
import {
  omitSession,
  promoteSessionSnapshots,
  shouldEvictSession,
} from '../lib/promoteSessionSnapshots';
import { useHubReconnect } from './useHubReconnect';

export interface SessionSnapshots {
  /** sessionId → ambient state, sourced from claudemon. */
  statusBySession: Record<string, SessionAmbientState>;
  /** sessionId → full snapshot. The shared substrate the Triage Inbox and
   *  Fleet Deck both project from. */
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
  /** Forget a session's promoted entries — call after terminating an agent. */
  pruneSession: (sessionId: string | undefined) => void;
  /**
   * Daemon sessions that were already alive when Workspacer launched.
   *
   * claudemon outlives the app, so these are leftovers from a previous run and
   * must NOT be auto-adopted as orphan cards — the user reaches them only by
   * explicitly resuming a saved session. `null` until the first session list
   * resolves, so adoption waits rather than guessing empty.
   */
  preexistingSessionIdsRef: React.MutableRefObject<Set<string> | null>;
  /** Re-pull the whole list. Runs at mount and on every hub reconnect. */
  refreshSessionSnapshots: () => void;
}

/**
 * The promoted per-session status/snapshot maps and their lifecycle.
 *
 * Three things keep these maps current, and they have to agree about which
 * sessions count (see `lib/promoteSessionSnapshots`):
 *  - a full pull at mount and on every hub reconnect, because while the socket
 *    is down we miss update ticks and a web tab would otherwise show stale or
 *    missing sessions until a manual refresh;
 *  - the live `onClaudeSessionUpdate` subscription; and
 *  - `pruneSession`, for agents the user terminates.
 *
 * @param stopAgentForSession flips the owning agent to stopped when its session
 *   ends on its own, so the card offers a respawn straight away.
 */
export function useSessionSnapshots(
  stopAgentForSession: (sessionId: string) => void,
): SessionSnapshots {
  const [statusBySession, setStatusBySession] = useState<Record<string, SessionAmbientState>>({});
  const [snapshotBySession, setSnapshotBySession] = useState<Record<string, ClaudeSessionSnapshot>>(
    {},
  );
  const preexistingSessionIdsRef = useRef<Set<string> | null>(null);

  const refreshSessionSnapshots = useCallback(() => {
    window.electronAPI
      .getAllClaudeSessions()
      .then((sessions: any[]) => {
        const { statusBySession: map, snapshotBySession: snaps } =
          promoteSessionSnapshots(sessions);
        if (preexistingSessionIdsRef.current === null) {
          preexistingSessionIdsRef.current = new Set(sessions.map((s) => s.sessionId));
        }
        setStatusBySession(map);
        setSnapshotBySession(snaps);
      })
      .catch(() => {
        // No daemon / empty list: nothing pre-existed, so adoption can proceed.
        if (preexistingSessionIdsRef.current === null) {
          preexistingSessionIdsRef.current = new Set();
        }
      });
  }, []);

  useHubReconnect(refreshSessionSnapshots);

  useEffect(() => {
    refreshSessionSnapshots();
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId: string, snapshot: any) => {
      if (shouldEvictSession(sessionId, snapshot.status)) {
        setStatusBySession((prev) => omitSession(prev, sessionId));
        setSnapshotBySession((prev) => omitSession(prev, sessionId));
        // No-op after an explicit terminate — the agent is already gone by the
        // time its session reports ended.
        stopAgentForSession(sessionId);
        return;
      }
      setStatusBySession((prev) => ({ ...prev, [sessionId]: snapshot.ambientState }));
      setSnapshotBySession((prev) => ({
        ...prev,
        [sessionId]: compactClaudeSnapshotForBackground(snapshot),
      }));
    });
    return () => {
      unsub();
    };
  }, [refreshSessionSnapshots, stopAgentForSession]);

  // useAgentManager.terminateAgent removes the agent + closes the daemon
  // session but doesn't own these maps, so without this they'd hold the dead
  // session's full transcript for the rest of the app's lifetime.
  const pruneSession = useCallback((sessionId: string | undefined) => {
    if (!sessionId) return;
    setStatusBySession((prev) => omitSession(prev, sessionId));
    setSnapshotBySession((prev) => omitSession(prev, sessionId));
  }, []);

  return {
    statusBySession,
    snapshotBySession,
    pruneSession,
    preexistingSessionIdsRef,
    refreshSessionSnapshots,
  };
}
