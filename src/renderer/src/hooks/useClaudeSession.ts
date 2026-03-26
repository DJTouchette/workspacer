import { useState, useEffect, useRef, useCallback } from 'react';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';

interface UseClaudeSessionOptions {
  /** The PTY session ID this hook tracks */
  ptySessionId: string | null;
}

interface UseClaudeSessionReturn {
  /** Latest session snapshot (null until first hook event arrives) */
  session: ClaudeSessionSnapshot | null;
  /** Force-refresh from main process */
  refresh: () => void;
}

/**
 * React hook that subscribes to Claude session state updates for a given PTY.
 * State is pushed from the main process via IPC whenever hook events fire
 * or ambient state changes.
 */
export function useClaudeSession({ ptySessionId }: UseClaudeSessionOptions): UseClaudeSessionReturn {
  const [session, setSession] = useState<ClaudeSessionSnapshot | null>(null);
  const ptyIdRef = useRef(ptySessionId);
  ptyIdRef.current = ptySessionId;

  // Subscribe to push updates from main process
  useEffect(() => {
    const unsub = window.electronAPI.onClaudeSessionUpdate((ptyId, snapshot) => {
      if (ptyId === ptyIdRef.current || snapshot.ptyId === ptyIdRef.current) {
        setSession(snapshot as ClaudeSessionSnapshot);
      }
    });
    return unsub;
  }, []);

  // Initial fetch when PTY ID becomes available
  useEffect(() => {
    if (!ptySessionId) return;
    window.electronAPI.getClaudeSessionByPty(ptySessionId).then((snap) => {
      if (snap) setSession(snap as ClaudeSessionSnapshot);
    });
  }, [ptySessionId]);

  const refresh = useCallback(() => {
    if (!ptyIdRef.current) return;
    window.electronAPI.getClaudeSessionByPty(ptyIdRef.current).then((snap) => {
      if (snap) setSession(snap as ClaudeSessionSnapshot);
    });
  }, []);

  return { session, refresh };
}
