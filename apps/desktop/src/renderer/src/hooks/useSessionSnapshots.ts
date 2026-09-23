import { clearSessionChatUiState } from './useSessionChatUiState';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ClaudeSessionSnapshot, SessionAmbientState } from '../types/claudeSession';
import { compactClaudeSnapshotForBackground } from '../lib/compactClaudeSnapshot';
import {
  omitSession,
  promoteSessionSnapshots,
  shouldEvictSession,
  type PromotedSessionMaps,
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

/** Routine streaming updates share one fleet publication, independent of fleet size. */
export const FLEET_SNAPSHOT_FLUSH_MS = 100;

function decisionKey(snapshot: ClaudeSessionSnapshot): string {
  return JSON.stringify([
    snapshot.status,
    snapshot.ambientState,
    snapshot.hubOffline,
    snapshot.pendingApproval ?? null,
    snapshot.pendingQuestions ?? null,
  ]);
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
  const [{ statusBySession, snapshotBySession }, setMaps] = useState<PromotedSessionMaps>({
    statusBySession: {},
    snapshotBySession: {},
  });
  const pending = useRef(new Map<string, ClaudeSessionSnapshot>());
  const decisions = useRef(new Map<string, string>());
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const refreshChanges = useRef<Set<string> | null>(null);
  const refreshGeneration = useRef(0);
  const preexistingSessionIdsRef = useRef<Set<string> | null>(null);

  const flush = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    if (!pending.current.size) return;
    const updates = [...pending.current];
    pending.current.clear();
    setMaps((prev) => {
      let statuses = prev.statusBySession;
      const snapshots = { ...prev.snapshotBySession };
      for (const [id, snapshot] of updates) {
        snapshots[id] = compactClaudeSnapshotForBackground(snapshot);
        if (statuses[id] !== snapshot.ambientState) {
          if (statuses === prev.statusBySession) statuses = { ...statuses };
          statuses[id] = snapshot.ambientState;
        }
      }
      return { statusBySession: statuses, snapshotBySession: snapshots };
    });
  }, []);

  const pruneSession = useCallback((sessionId: string | undefined) => {
    if (!sessionId) return;
    refreshChanges.current?.add(sessionId);
    pending.current.delete(sessionId);
    decisions.current.delete(sessionId);
    clearSessionChatUiState(sessionId);
    setMaps((prev) => {
      const statuses = omitSession(prev.statusBySession, sessionId);
      const snapshots = omitSession(prev.snapshotBySession, sessionId);
      return statuses === prev.statusBySession && snapshots === prev.snapshotBySession
        ? prev
        : { statusBySession: statuses, snapshotBySession: snapshots };
    });
  }, []);

  const refreshSessionSnapshots = useCallback(() => {
    // Drain observations received before this authoritative read. Otherwise
    // their timer could publish stale state after the newer list resolves.
    flush();
    const generation = ++refreshGeneration.current;
    refreshChanges.current = new Set();
    window.electronAPI
      .getAllClaudeSessions()
      .then((sessions: any[]) => {
        if (generation !== refreshGeneration.current) return;
        const changed = refreshChanges.current ?? new Set<string>();
        refreshChanges.current = null;
        const { statusBySession: map, snapshotBySession: snaps } =
          promoteSessionSnapshots(sessions);
        if (preexistingSessionIdsRef.current === null) {
          preexistingSessionIdsRef.current = new Set(sessions.map((s) => s.sessionId));
        }
        for (const id of decisions.current.keys()) {
          if (!(id in snaps) && !changed.has(id)) decisions.current.delete(id);
        }
        for (const [id, snapshot] of Object.entries(snaps)) {
          if (!changed.has(id)) decisions.current.set(id, decisionKey(snapshot));
        }
        setMaps((prev) => {
          // A reconnect fetch must not overwrite a newer push or resurrect a
          // session that ended while the list request was in flight.
          for (const id of changed) {
            if (prev.snapshotBySession[id]) {
              snaps[id] = prev.snapshotBySession[id];
              map[id] = prev.statusBySession[id];
            } else {
              delete snaps[id];
              delete map[id];
            }
          }
          return { statusBySession: map, snapshotBySession: snaps };
        });
      })
      .catch(() => {
        if (generation !== refreshGeneration.current) return;
        refreshChanges.current = null;
        // No daemon / empty list: nothing pre-existed, so adoption can proceed.
        if (preexistingSessionIdsRef.current === null) {
          preexistingSessionIdsRef.current = new Set();
        }
      });
  }, [flush]);

  useHubReconnect(refreshSessionSnapshots);

  useEffect(() => {
    refreshSessionSnapshots();
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId: string, snapshot: any) => {
      if (shouldEvictSession(sessionId, snapshot.status)) {
        pruneSession(sessionId);
        // No-op after an explicit terminate — the agent is already gone by the
        // time its session reports ended.
        stopAgentForSession(sessionId);
        return;
      }
      refreshChanges.current?.add(sessionId);
      const key = decisionKey(snapshot);
      const urgent = decisions.current.get(sessionId) !== key;
      decisions.current.set(sessionId, key);
      pending.current.set(sessionId, snapshot);
      // New sessions, state transitions and changed decisions bypass the timer.
      // Flush older queued observations too, preserving their event ordering.
      if (urgent) flush();
      else if (!timer.current) timer.current = setTimeout(flush, FLEET_SNAPSHOT_FLUSH_MS);
    });
    return () => {
      unsub();
      refreshGeneration.current++;
      refreshChanges.current = null;
      if (timer.current) clearTimeout(timer.current);
      timer.current = null;
      pending.current.clear();
    };
  }, [refreshSessionSnapshots, stopAgentForSession, flush, pruneSession]);

  return {
    statusBySession,
    snapshotBySession,
    pruneSession,
    preexistingSessionIdsRef,
    refreshSessionSnapshots,
  };
}
