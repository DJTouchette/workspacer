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
  /** Called when the PTY process exits */
  onExit?: () => void;
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
}

/**
 * React hook that manages a single PTY session lifecycle.
 *
 * - Creates a PTY session on mount via the Electron main process
 * - Subscribes to output/exit events via IPC
 * - Provides write/resize functions to interact with the PTY
 * - Cleans up (closes PTY, unsubscribes) on unmount
 */
export function usePTY({ paneId, shell = '', cwd, onExit }: UsePTYOptions): UsePTYReturn {
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

  // Output batching: collect chunks and flush once per animation frame
  const pendingOutputRef = useRef<Uint8Array[]>([]);
  const rafRef = useRef<number | null>(null);

  const attachToTerminal = useCallback((term: Terminal) => {
    termRef.current = term;
  }, []);

  const write = useCallback((data: string) => {
    const id = sessionIdRef.current;
    if (!id) return;
    // btoa handles ASCII/latin1 directly which covers 99% of terminal input
    const encoded = btoa(data);
    WriteTerminal(id, encoded).catch((err) => {
      console.error('[usePTY] write error:', err);
    });
  }, []);

  const resize = useCallback((cols: number, rows: number) => {
    const id = sessionIdRef.current;
    if (!id) return;
    ResizeTerminal(id, cols, rows).catch((err) => {
      console.error('[usePTY] resize error:', err);
    });
  }, []);

  // Main effect: create PTY session on mount, subscribe to events, clean up on unmount
  useEffect(() => {
    mountedRef.current = true;
    let localSessionId: string | null = null;
    let unsubOutput: (() => void) | null = null;
    let unsubExit: (() => void) | null = null;

    const init = async () => {
      try {
        const id = await CreateTerminal(shellRef.current, cwdRef.current);
        if (!mountedRef.current) {
          // Component unmounted before we got the session — close it immediately
          CloseTerminal(id).catch(() => {});
          return;
        }

        localSessionId = id;
        sessionIdRef.current = id;
        setSessionId(id);

        // Subscribe to PTY output events — filter by session ID, batch for performance
        unsubOutput = window.electronAPI.onTerminalOutput((eventId, base64Data) => {
          if (eventId !== id) return;
          if (!base64Data || !termRef.current) return;

          try {
            const raw = atob(base64Data);
            const bytes = new Uint8Array(raw.length);
            for (let i = 0; i < raw.length; i++) {
              bytes[i] = raw.charCodeAt(i);
            }
            // Queue the chunk and flush on next animation frame
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
          } catch (err) {
            console.error('[usePTY] output decode error:', err);
          }
        });

        // Subscribe to PTY exit events
        unsubExit = window.electronAPI.onTerminalExit((eventId) => {
          if (eventId !== id) return;
          if (onExitRef.current) {
            onExitRef.current();
          }
        });

        setIsReady(true);
      } catch (err) {
        console.error('[usePTY] failed to create terminal:', err);
      }
    };

    init();

    return () => {
      mountedRef.current = false;

      // Cancel pending output flush
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      pendingOutputRef.current = [];

      // Unsubscribe from events
      if (unsubOutput) unsubOutput();
      if (unsubExit) unsubExit();

      // Close the PTY session
      if (localSessionId) {
        CloseTerminal(localSessionId).catch((err) => {
          console.error('[usePTY] close error:', err);
        });
      }

      sessionIdRef.current = null;
    };
  // Only re-run when paneId changes — shell is read from ref at creation time
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paneId]);

  return {
    sessionId,
    isReady,
    write,
    resize,
    attachToTerminal,
  };
}
