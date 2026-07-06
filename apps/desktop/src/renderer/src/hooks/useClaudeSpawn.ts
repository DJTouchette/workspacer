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

/** Settings overrides for restartSession (composer pills' restart path). */
export interface RestartSessionOverrides {
  provider?: string;
  /** Claude only: keep the session on its current transport ('pty' | 'stream')
   *  across the restart, instead of falling back to the config default. */
  transport?: 'pty' | 'stream';
  model?: string;
  effort?: string;
  permissionMode?: string;
}

interface UseClaudeSpawnReturn {
  /** claudemon canonical session_id (null until spawn resolves) */
  sessionId: string | null;
  isReady: boolean;
  /** Set when spawn/attach failed; null while pending or on success. */
  spawnError: Error | null;
  /** Write raw bytes/string straight to claude's stdin via MessagePort. */
  write: (data: string) => void;
  /** Resize the session's PTY (debounced). */
  resize: (cols: number, rows: number) => void;
  attachToTerminal: (term: Terminal) => void;
  /** Manually spawn the claude session at known dimensions (defer mode). */
  startSession: (cols: number, rows: number) => void;
  /** Re-run the init/start path after a failed spawn (clears spawnError). */
  retry: () => void;
  /** Owner panes only: close the session and respawn it (resuming the same
   *  pinned id) with new launch settings. No-op for attached viewers — their
   *  restart goes through the agent manager (see ClaudePane). */
  restartSession: (overrides: RestartSessionOverrides) => Promise<void>;
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
  const [spawnError, setSpawnError] = useState<Error | null>(null);
  /** Last dimensions passed to startSession, so retry can re-spawn at the
   *  same size without the caller re-supplying them. */
  const lastDimsRef = useRef<{ cols?: number; rows?: number }>({});
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
  const unsubExitRef = useRef<(() => void) | null>(null);
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

  /** Subscribe the exit + output streams for a viewer key (spawn/attach/restart
   *  all share this wiring). */
  const subscribeStreams = useCallback((viewerKey: string) => {
    // Session death: main emits terminal:exit keyed by the sessionId (owner
    // stream 404) or the viewerKey (dead attach target). Match either so both
    // owner and attached panes learn their session is gone. Without this
    // subscription onExit was never invoked at all — panes rendered a dead
    // session indistinguishably from a live idle one.
    unsubExitRef.current = window.electronAPI.onTerminalExit((eventId: string) => {
      if (eventId !== sessionIdRef.current && eventId !== viewerKeyRef.current) return;
      onExitRef.current?.();
    });

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
            try {
              term.write(chunk);
            } catch (err) {
              console.warn('[useClaudeSpawn] term.write threw:', err);
            }
          }
        });
      }
    });
  }, []);

  const initSession = useCallback(async (cols?: number, rows?: number) => {
    lastDimsRef.current = { cols, rows };
    setSpawnError(null);
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
      subscribeStreams(viewerKey);
      setIsReady(true);
    } catch (err) {
      console.error('[useClaudeSpawn] spawn failed:', err);
      if (mountedRef.current) {
        setSpawnError(err instanceof Error ? err : new Error(String(err)));
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /**
   * Owner-pane restart: close the live session, respawn resuming the SAME
   * pinned id with new launch settings (model/effort/permission mode), and
   * rewire the byte streams. The daemon returns the same canonical id on a
   * resume spawn, so the GUI snapshot stream (keyed by session id) is
   * continuous across the restart. Attached viewers must not use this — their
   * session belongs to the agent manager (respawn goes through it instead).
   */
  const restartSession = useCallback(
    async (overrides: RestartSessionOverrides) => {
      const old = sessionIdRef.current;
      if (!old || isAttachedRef.current) return;
      if (unsubOutputRef.current) {
        unsubOutputRef.current();
        unsubOutputRef.current = null;
      }
      if (unsubExitRef.current) {
        unsubExitRef.current();
        unsubExitRef.current = null;
      }
      await window.electronAPI.claudeClose(old).catch(() => {});
      sessionIdRef.current = null;
      viewerKeyRef.current = null;
      setIsReady(false);
      setSpawnError(null);
      const { cols, rows } = lastDimsRef.current;
      try {
        const id = await window.electronAPI.spawnClaude({
          cwd: cwdRef.current,
          profileId: profileRef.current,
          provider: overrides.provider as 'claude' | 'codex' | 'opencode' | 'pi' | undefined,
          transport: overrides.transport,
          resumeSessionId: old,
          model: overrides.model,
          effort: overrides.effort,
          permissionMode: overrides.permissionMode,
          cols,
          rows,
        });
        if (!mountedRef.current) {
          window.electronAPI.claudeClose(id).catch(() => {});
          return;
        }
        sessionIdRef.current = id;
        viewerKeyRef.current = id;
        setSessionId(id);
        subscribeStreams(id);
        setIsReady(true);
      } catch (err) {
        console.error('[useClaudeSpawn] restart failed:', err);
        if (mountedRef.current) {
          setSpawnError(err instanceof Error ? err : new Error(String(err)));
        }
      }
    },
    [subscribeStreams],
  );

  const startSession = useCallback(
    (cols: number, rows: number) => {
      if (sessionIdRef.current) return;
      initSession(cols, rows);
    },
    [initSession],
  );

  const retry = useCallback(() => {
    if (sessionIdRef.current) return;
    const { cols, rows } = lastDimsRef.current;
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
      if (unsubExitRef.current) unsubExitRef.current();
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
    spawnError,
    write,
    resize,
    attachToTerminal,
    startSession,
    retry,
    restartSession,
  };
}
