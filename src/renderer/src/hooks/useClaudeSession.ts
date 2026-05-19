import { useState, useEffect, useRef, useCallback } from 'react';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';

interface UseClaudeSessionOptions {
  /** The claudemon session_id this hook tracks (formerly the PTY id) */
  ptySessionId: string | null;
}

interface UseClaudeSessionReturn {
  session: ClaudeSessionSnapshot | null;
  refresh: () => void;
}

/**
 * Subscribes to Claude session state updates pushed from the main process.
 * Updates are emitted by `claudeSessionStore` whenever a hook event arrives.
 */
export function useClaudeSession({ ptySessionId }: UseClaudeSessionOptions): UseClaudeSessionReturn {
  const [session, setSession] = useState<ClaudeSessionSnapshot | null>(null);
  const idRef = useRef(ptySessionId);
  idRef.current = ptySessionId;

  useEffect(() => {
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId, snapshot) => {
      if (sessionId === idRef.current || snapshot.sessionId === idRef.current) {
        setSession(snapshot as ClaudeSessionSnapshot);
      }
    });
    return unsub;
  }, []);

  useEffect(() => {
    if (!ptySessionId) return;
    window.electronAPI.getClaudeSession(ptySessionId).then((snap) => {
      if (snap) setSession(snap as ClaudeSessionSnapshot);
    });
  }, [ptySessionId]);

  const refresh = useCallback(() => {
    if (!idRef.current) return;
    window.electronAPI.getClaudeSession(idRef.current).then((snap) => {
      if (snap) setSession(snap as ClaudeSessionSnapshot);
    });
  }, []);

  return { session, refresh };
}
