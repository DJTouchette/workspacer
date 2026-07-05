import { useState, useEffect, useRef, useCallback } from 'react';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';

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
  const idRef = useRef(ptySessionId);
  idRef.current = ptySessionId;

  // Coalescing state for the inactive case: hold the newest snapshot and flush
  // it on a slow timer so status stays roughly live without per-token renders.
  const activeRef = useRef(active);
  const pendingRef = useRef<ClaudeSessionSnapshot | null>(null);
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // When the pane becomes active again, flush the latest snapshot immediately
  // so the user never sees stale state on the pane they just navigated to.
  useEffect(() => {
    activeRef.current = active;
    if (active && pendingRef.current) {
      setSession(pendingRef.current);
      pendingRef.current = null;
      if (flushTimerRef.current) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
    }
  }, [active]);

  useEffect(() => {
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId, snapshot) => {
      if (sessionId === idRef.current || snapshot.sessionId === idRef.current) {
        const snap = snapshot as ClaudeSessionSnapshot;
        if (activeRef.current) {
          setSession(snap);
          return;
        }
        // Off-screen: keep only the newest snapshot, flushed on a slow cadence.
        pendingRef.current = snap;
        if (!flushTimerRef.current) {
          flushTimerRef.current = setTimeout(() => {
            flushTimerRef.current = null;
            if (pendingRef.current) {
              setSession(pendingRef.current);
              pendingRef.current = null;
            }
          }, INACTIVE_FLUSH_MS);
        }
      }
    });
    return () => {
      unsub();
      if (flushTimerRef.current) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
    };
  }, []);

  // Incremented each time ptySessionId changes so a stale async response
  // (from an earlier id) is silently dropped instead of overwriting newer state.
  const generationRef = useRef(0);

  useEffect(() => {
    // The tracked id changed (or cleared). Drop the previous session's snapshot
    // immediately — otherwise a pane re-pointed at a session that has no
    // snapshot yet, or detached to null, keeps rendering the old session's
    // state (its status, pending prompts, etc.). The fetch below repopulates
    // when the new id does have a snapshot.
    setSession(null);
    pendingRef.current = null;
    if (!ptySessionId) return;
    const gen = ++generationRef.current;
    window.electronAPI.getClaudeSession(ptySessionId).then((snap) => {
      if (gen !== generationRef.current) return; // stale or unmounted
      if (snap) setSession(snap as ClaudeSessionSnapshot);
    });
    return () => {
      // Increment so any in-flight promise from this ptySessionId is ignored
      generationRef.current++;
    };
  }, [ptySessionId]);

  const refresh = useCallback(() => {
    if (!idRef.current) return;
    const gen = ++generationRef.current;
    window.electronAPI.getClaudeSession(idRef.current).then((snap) => {
      if (gen !== generationRef.current) return; // stale or unmounted
      if (snap) setSession(snap as ClaudeSessionSnapshot);
    });
  }, []);

  return { session, refresh };
}
