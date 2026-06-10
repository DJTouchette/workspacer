/**
 * Claude session lifecycle hook — analogous to `usePTY` but routes through
 * the claudemon daemon via main-process IPC. Workspacer renderer no longer
 * owns a PTY for Claude panes; xterm here is a thin viewer over the
 * claudemon byte stream.
 */

import { useEffect, useRef, useCallback, useState } from 'react';
import type { Terminal } from '@xterm/xterm';
import { binaryStringToUint8Array } from '../lib/terminalUtils';

interface UseClaudeSpawnOptions {
  paneId: string;
  cwd?: string;
  profileId?: string;
  resumeSessionId?: string;
  /** If set, attach as a viewer to an already-running daemon session instead
   *  of spawning. The pane will subscribe to the byte stream without starting
   *  a new Claude process. */
  attachSessionId?: string;
  onExit?: () => void;
  /** If true, don't spawn on mount — call startSession() manually. */
  defer?: boolean;
}

interface UseClaudeSpawnReturn {
  /** claudemon canonical session_id (null until spawn resolves) */
  sessionId: string | null;
  isReady: boolean;
  /** Write raw bytes/string straight to claude's stdin via MessagePort. */
  write: (data: string) => void;
  /** Resize the session's PTY (debounced). */
  resize: (cols: number, rows: number) => void;
  attachToTerminal: (term: Terminal) => void;
  /** Manually spawn the claude session at known dimensions (defer mode). */
  startSession: (cols: number, rows: number) => void;
}

export function useClaudeSpawn({
  paneId,
  cwd,
  profileId,
  resumeSessionId,
  attachSessionId,
  onExit,
  defer = false,
}: UseClaudeSpawnOptions): UseClaudeSpawnReturn {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [isReady, setIsReady] = useState(false);
  /** True if this pane attached to an existing session (vs spawned its own).
   *  Determines unmount cleanup: attached panes detach, owners close (SIGTERM). */
  const isAttachedRef = useRef(false);

  const termRef = useRef<Terminal | null>(null);
  const mountedRef = useRef(true);
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;

  const sessionIdRef = useRef<string | null>(null);
  const cwdRef = useRef(cwd);
  cwdRef.current = cwd;
  const profileRef = useRef(profileId);
  profileRef.current = profileId;
  const resumeRef = useRef(resumeSessionId);
  resumeRef.current = resumeSessionId;
  const attachRef = useRef(attachSessionId);
  attachRef.current = attachSessionId;

  const pendingOutputRef = useRef<Uint8Array[]>([]);
  const rafRef = useRef<number | null>(null);
  const unsubOutputRef = useRef<(() => void) | null>(null);
  /** Key for port lookup in the preload — paneId for attached viewers,
   *  sessionId for spawned (owner) panes. */
  const viewerKeyRef = useRef<string | null>(null);

  const attachToTerminal = useCallback((term: Terminal) => {
    termRef.current = term;
  }, []);

  const write = useCallback((data: string) => {
    const key = viewerKeyRef.current;
    if (!key) return;
    window.electronAPI.claudeWrite(key, data);
  }, []);

  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const resize = useCallback((cols: number, rows: number) => {
    const id = sessionIdRef.current;
    if (!id) return;
    if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
    resizeTimerRef.current = setTimeout(() => {
      resizeTimerRef.current = null;
      window.electronAPI.claudeResize(id, cols, rows).catch((err) => {
        console.error('[useClaudeSpawn] resize error:', err);
      });
    }, 50);
  }, []);

  const initSession = useCallback(async (cols?: number, rows?: number) => {
    try {
      let id: string;
      let viewerKey: string;
      if (attachRef.current) {
        // Attach mode — subscribe as a viewer to an already-running session.
        // The viewerKey is the paneId so multiple viewers of the same session
        // can each have their own port.
        id = await window.electronAPI.attachClaude(paneId, attachRef.current);
        viewerKey = paneId;
        isAttachedRef.current = true;
      } else {
        id = await window.electronAPI.spawnClaude({
          cwd: cwdRef.current,
          profileId: profileRef.current,
          resumeSessionId: resumeRef.current,
          cols,
          rows,
        });
        viewerKey = id;
        isAttachedRef.current = false;
      }
      if (!mountedRef.current) {
        if (isAttachedRef.current) {
          window.electronAPI.detachClaude(paneId).catch(() => {});
        } else {
          window.electronAPI.claudeClose(id).catch(() => {});
        }
        return;
      }
      sessionIdRef.current = id;
      viewerKeyRef.current = viewerKey;
      setSessionId(id);

      unsubOutputRef.current = window.electronAPI.onClaudeOutput(viewerKey, (data: string) => {
        if (!data || !termRef.current) return;
        pendingOutputRef.current.push(binaryStringToUint8Array(data));
        if (rafRef.current === null) {
          rafRef.current = requestAnimationFrame(() => {
            rafRef.current = null;
            const term = termRef.current;
            if (!term) return;
            const chunks = pendingOutputRef.current;
            pendingOutputRef.current = [];
            // Guard each write: a single malformed/edge-case sequence that makes
            // xterm throw must not abort the batch or break the render loop.
            for (const chunk of chunks) {
              try { term.write(chunk); } catch (err) { console.warn('[useClaudeSpawn] term.write threw:', err); }
            }
          });
        }
      });

      setIsReady(true);
    } catch (err) {
      console.error('[useClaudeSpawn] spawn failed:', err);
    }
  }, []);

  const startSession = useCallback((cols: number, rows: number) => {
    if (sessionIdRef.current) return;
    initSession(cols, rows);
  }, [initSession]);

  useEffect(() => {
    mountedRef.current = true;
    if (!defer) initSession();
    return () => {
      mountedRef.current = false;
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      if (resizeTimerRef.current !== null) {
        clearTimeout(resizeTimerRef.current);
        resizeTimerRef.current = null;
      }
      pendingOutputRef.current = [];
      if (unsubOutputRef.current) unsubOutputRef.current();
      if (sessionIdRef.current) {
        if (isAttachedRef.current) {
          window.electronAPI.detachClaude(paneId).catch((err) => {
            console.error('[useClaudeSpawn] detach error:', err);
          });
        } else {
          window.electronAPI.claudeClose(sessionIdRef.current).catch((err) => {
            console.error('[useClaudeSpawn] close error:', err);
          });
        }
      }
      sessionIdRef.current = null;
      viewerKeyRef.current = null;
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paneId]);

  return {
    sessionId,
    isReady,
    write,
    resize,
    attachToTerminal,
    startSession,
  };
}
