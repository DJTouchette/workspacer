import { useEffect, useRef, useCallback, useState } from 'react';
import type { Terminal } from '@xterm/xterm';
import {
  CreateTerminal,
  WriteTerminal,
  ResizeTerminal,
  CloseTerminal,
} from '../lib/terminalApi';

interface UsePTYOptions {
  /** The pane ID this terminal belongs to */
  paneId: string;
  /** Shell to launch (empty string = system default) */
  shell?: string;
  /** Initial working directory */
  cwd?: string;
  /** Claude profile ID (sets CLAUDE_CONFIG_DIR + extra args) */
  profileId?: string;
  /** Claude session ID to resume (--resume flag) */
  resumeSessionId?: string;
  /** Called when the PTY process exits */
  onExit?: () => void;
  /** If true, don't create PTY on mount — call startPTY() manually */
  defer?: boolean;
}

interface UsePTYReturn {
  sessionId: string | null;
  isReady: boolean;
  /** Send user input to the PTY (raw string, will be base64-encoded) */
  write: (data: string) => void;
  /** Resize the PTY to the given dimensions */
  resize: (cols: number, rows: number) => void;
  /** Connect an xterm.js Terminal instance so PTY output is written to it */
  attachToTerminal: (term: Terminal) => void;
  /** Create the PTY with specific dimensions (only when defer=true) */
  startPTY: (cols: number, rows: number) => void;
}

/**
 * React hook that manages a single PTY session lifecycle.
 *
 * - Creates a PTY session on mount (or deferred via startPTY)
 * - Subscribes to output/exit events via IPC
 * - Provides write/resize functions to interact with the PTY
 * - Cleans up (closes PTY, unsubscribes) on unmount
 */
export function usePTY({ paneId, shell = '', cwd, profileId, resumeSessionId, onExit, defer = false }: UsePTYOptions): UsePTYReturn {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [isReady, setIsReady] = useState(false);

  // Store the xterm Terminal reference without triggering re-renders
  const termRef = useRef<Terminal | null>(null);
  // Track whether the component is still mounted
  const mountedRef = useRef(true);
  // Store the onExit callback in a ref so it's always current
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;

  // Store sessionId in a ref for use in callbacks that shouldn't retrigger effects
  const sessionIdRef = useRef<string | null>(null);
  // Store shell/cwd in refs so config changes don't tear down live sessions
  const shellRef = useRef(shell);
  shellRef.current = shell;
  const cwdRef = useRef(cwd);
  cwdRef.current = cwd;
  const profileIdRef = useRef(profileId);
  profileIdRef.current = profileId;
  const resumeSessionIdRef = useRef(resumeSessionId);
  resumeSessionIdRef.current = resumeSessionId;

  // Output batching: collect chunks and flush once per animation frame
  const pendingOutputRef = useRef<Uint8Array[]>([]);
  const rafRef = useRef<number | null>(null);
  // Cleanup refs for deferred mode
  const unsubOutputRef = useRef<(() => void) | null>(null);
  const unsubExitRef = useRef<(() => void) | null>(null);

  const attachToTerminal = useCallback((term: Terminal) => {
    termRef.current = term;
  }, []);

  const write = useCallback((data: string) => {
    const id = sessionIdRef.current;
    if (!id) return;
    WriteTerminal(id, data);
  }, []);

  // Debounce resize to avoid flooding IPC during window drag
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const resize = useCallback((cols: number, rows: number) => {
    const id = sessionIdRef.current;
    if (!id) return;
    if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
    resizeTimerRef.current = setTimeout(() => {
      resizeTimerRef.current = null;
      ResizeTerminal(id, cols, rows).catch((err) => {
        console.error('[usePTY] resize error:', err);
      });
    }, 50);
  }, []);

  // Core init logic shared by auto and deferred modes
  const initPTY = useCallback(async (cols?: number, rows?: number) => {
    try {
      const id = await CreateTerminal(shellRef.current, cwdRef.current, cols, rows, profileIdRef.current, resumeSessionIdRef.current);
      if (!mountedRef.current) {
        CloseTerminal(id).catch(() => {});
        return;
      }

      sessionIdRef.current = id;
      setSessionId(id);

      // Subscribe to PTY output via MessagePort (no base64, no IPC dispatch)
      unsubOutputRef.current = window.electronAPI.onTerminalOutput(id, (data: string) => {
        if (!data || !termRef.current) return;

        // Convert binary string to Uint8Array for xterm
        const bytes = new Uint8Array(data.length);
        for (let i = 0; i < data.length; i++) {
          bytes[i] = data.charCodeAt(i);
        }
        pendingOutputRef.current.push(bytes);
        if (rafRef.current === null) {
          rafRef.current = requestAnimationFrame(() => {
            rafRef.current = null;
            const term = termRef.current;
            if (!term) return;
            const chunks = pendingOutputRef.current;
            pendingOutputRef.current = [];
            for (const chunk of chunks) {
              term.write(chunk);
            }
          });
        }
      });

      unsubExitRef.current = window.electronAPI.onTerminalExit((eventId) => {
        if (eventId !== id) return;
        if (onExitRef.current) {
          onExitRef.current();
        }
      });

      setIsReady(true);
    } catch (err) {
      console.error('[usePTY] failed to create terminal:', err);
    }
  }, []);

  /** Manually start PTY with known dimensions (for defer mode) */
  const startPTY = useCallback((cols: number, rows: number) => {
    if (sessionIdRef.current) return; // already started
    initPTY(cols, rows);
  }, [initPTY]);

  // Auto-create PTY on mount (unless deferred)
  useEffect(() => {
    mountedRef.current = true;

    if (!defer) {
      initPTY();
    }

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
        CloseTerminal(sessionIdRef.current).catch((err) => {
          console.error('[usePTY] close error:', err);
        });
      }

      sessionIdRef.current = null;
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paneId]);

  return {
    sessionId,
    isReady,
    write,
    resize,
    attachToTerminal,
    startPTY,
  };
}
