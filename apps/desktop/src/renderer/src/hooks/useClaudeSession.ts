import { useState, useEffect, useRef, useCallback } from 'react';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { compactClaudeSnapshotForBackground } from '../lib/compactClaudeSnapshot';
import { reconcileConversationTurns } from '../lib/conversationIndex';

interface UseClaudeSessionOptions {
  /** The claudemon session_id this hook tracks (formerly the PTY id) */
  ptySessionId: string | null;
  /**
   * Whether the owning pane is currently on-screen. When false we coalesce
   * incoming snapshots instead of re-rendering on every streamed token —
   * off-screen panes were a major source of scroll jank since every tab stays
   * mounted in the horizontal scroll container.
   */
  active?: boolean;
}

interface UseClaudeSessionReturn {
  session: ClaudeSessionSnapshot | null;
  refresh: () => void;
}

/** How often an inactive (off-screen) pane flushes its latest snapshot. */
const INACTIVE_FLUSH_MS = 1000;

/**
 * Subscribes to Claude session state updates pushed from the main process.
 * Updates are emitted by `claudeSessionStore` whenever a hook event arrives.
 */
export function useClaudeSession({
  ptySessionId,
  active = true,
}: UseClaudeSessionOptions): UseClaudeSessionReturn {
  const [session, setSession] = useState<ClaudeSessionSnapshot | null>(null);
  const reloadRef = useRef<() => void>(() => {});
  const applySnapshot = useCallback((snapshot: ClaudeSessionSnapshot) => {
    setSession((previous) => {
      if (
        !previous ||
        previous.sessionId !== snapshot.sessionId ||
        (previous.conversationOffset ?? 0) !== (snapshot.conversationOffset ?? 0)
      )
        return snapshot;
      return {
        ...snapshot,
        conversation: reconcileConversationTurns(
          previous.conversation,
          snapshot.conversation ?? [],
        ),
      };
    });
  }, []);

  useEffect(() => {
    let disposed = false;
    let revision = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let pending: ClaudeSessionSnapshot | null = null;
    setSession((previous) => {
      if (!ptySessionId || previous?.sessionId !== ptySessionId) return null;
      return active ? previous : compactClaudeSnapshotForBackground(previous);
    });
    if (!ptySessionId) {
      reloadRef.current = () => {};
      return;
    }
    const receive = (snapshot: ClaudeSessionSnapshot) => {
      if (disposed) return;
      revision++;
      if (active || snapshot.status === 'ended') {
        pending = null;
        if (timer) clearTimeout(timer);
        timer = undefined;
        applySnapshot(active ? snapshot : compactClaudeSnapshotForBackground(snapshot));
        return;
      }
      pending = compactClaudeSnapshotForBackground(snapshot);
      if (!timer)
        timer = setTimeout(() => {
          timer = undefined;
          if (!disposed && pending) applySnapshot(pending);
          pending = null;
        }, INACTIVE_FLUSH_MS);
    };
    // Only the direct IPC backend has a separate full-detail stream. The bus
    // backend retains its existing conversation-window/delta reconciliation.
    const detail = active && window.electronAPI.onClaudeSessionDetail;
    const unsubscribe = detail
      ? detail(ptySessionId, (snapshot) => receive(snapshot as ClaudeSessionSnapshot))
      : window.electronAPI.onClaudeSessionUpdate((id, snapshot) => {
          if (id === ptySessionId || snapshot.sessionId === ptySessionId)
            receive(snapshot as ClaudeSessionSnapshot);
        });
    const reload = () => {
      const requestedRevision = ++revision;
      window.electronAPI
        .getClaudeSession(ptySessionId, !active)
        .then((snapshot) => {
          // A later streamed update wins over an older in-flight full fetch.
          if (disposed || !snapshot) return;
          if (revision !== requestedRevision) {
            // A bus window can arrive before its initial full-history fetch.
            // Keep that newer window/metadata but recover only the missing
            // prefix; never replace its overlapping turns with older content.
            if (active && !detail)
              setSession((current) => {
                const full = snapshot as ClaudeSessionSnapshot;
                if (
                  !current ||
                  current.sessionId !== full.sessionId ||
                  current.executionEngine?.generation !== full.executionEngine?.generation
                )
                  return current;
                const start = current.conversationOffset ?? 0;
                const fullStart = full.conversationOffset ?? 0;
                const prefix = start - fullStart;
                if (prefix <= 0 || prefix > (full.conversation?.length ?? 0)) return current;
                return {
                  ...current,
                  conversation: [...full.conversation.slice(0, prefix), ...current.conversation],
                  conversationOffset: fullStart,
                  conversationUserOffset: full.conversationUserOffset,
                };
              });
            return;
          }
          pending = null;
          if (timer) clearTimeout(timer);
          timer = undefined;
          const next = snapshot as ClaudeSessionSnapshot;
          applySnapshot(active ? next : compactClaudeSnapshotForBackground(next));
        })
        .catch((error) => {
          if (!disposed) console.warn('[session] snapshot refresh failed', error);
        });
    };
    reloadRef.current = reload;
    reload();
    return () => {
      disposed = true;
      unsubscribe();
      if (timer) clearTimeout(timer);
      pending = null;
      reloadRef.current = () => {};
    };
  }, [ptySessionId, active, applySnapshot]);

  const refresh = useCallback(() => reloadRef.current(), []);
  return { session, refresh };
}
