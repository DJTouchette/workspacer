import { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebFontsAddon } from '@xterm/addon-web-fonts';
import '@xterm/xterm/css/xterm.css';
import { usePTY } from '../hooks/usePTY';
import { useConfig, Config } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { claudeColors as colors, ensureKeyframes } from '../components/claude-shared';
import {
  quoteFontFamily,
  fitWithRetry,
  isTermVisible,
  refitAndRepaint,
} from '../lib/terminalUtils';

/**
 * MEMORY: dispose the xterm.js instance (canvas + scrollback buffer) of a pane
 * that has been off-screen for longer than DISPOSE_HIDDEN_TERMINALS_MS, and
 * re-create it when it scrolls back into view. The React component and its DOM
 * container node STAY MOUNTED throughout; the backend PTY process is NOT killed
 * (no CloseTerminal on hide — only on real unmount, as today).
 *
 * DEFAULT: false. The clean re-show path requires the renderer to re-open the
 * PTY byte stream so claudemon replays its output ring buffer into the fresh
 * xterm. usePTY (which this lane does NOT own) keeps the SSE byte stream alive
 * across hide/show and exposes only `attachToTerminal(term)`, which just
 * repoints `termRef` for *future* live bytes — it does NOT re-subscribe, so the
 * daemon's replay snapshot is never re-fetched. Re-creating the xterm here would
 * therefore yield a BLANK terminal that only shows new output, losing all prior
 * scrollback/screen contents. Shipping that would violate the no-data-loss
 * constraint, so the behaviour is implemented but gated off by default.
 *
 * TO ENABLE (usePTY follow-up, separate lane): add a `resubscribe()` (or make
 * `attachToTerminal` re-open the byte stream via a fresh
 * `window.electronAPI.onTerminalOutput(id, ...)` / re-spawn of the SSE
 * consumer) so a new subscriber triggers claudemon's snapshot replay
 * (services/claudemon: stream_bytes -> snapshot_and_subscribe emits the output
 * ring buffer as the first SSE frame, then live bytes). Call that after the new
 * xterm is opened in recreateTerminal(), then flip this flag to true.
 */
const DISPOSE_HIDDEN_TERMINALS = false;
/** How long a pane must stay off-screen before its xterm is disposed. */
const DISPOSE_HIDDEN_TERMINALS_MS = 60_000;

interface TerminalPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  shell?: string;
  cwd?: string;
  /** Command typed into the PTY once it's ready (script buttons). */
  initialCommand?: string;
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
}

const TerminalPane: React.FC<TerminalPaneProps> = ({
  paneId,
  title,
  isActive,
  shell,
  cwd,
  initialCommand,
  onPtyReady,
}) => {
  const ranInitialRef = useRef(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const initializedRef = useRef(false);
  // Latest active state, readable from async init callbacks below.
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;

  // ── DISPOSE_HIDDEN_TERMINALS bookkeeping ──
  // Disposes the per-instance teardown for the live xterm (data/resize/binary
  // listeners + ResizeObserver) so we can tear it down on hide and rebuild on
  // show without unmounting the component.
  const instanceCleanupRef = useRef<(() => void) | null>(null);
  // True once the PTY has been started (first real terminal build). Re-creating
  // the xterm after a hide must NOT start a second PTY.
  const ptyStartedRef = useRef(false);
  // True while the xterm instance has been disposed for memory savings but the
  // component (and PTY) are still alive.
  const xtermDisposedRef = useRef(false);
  // Timer that fires once the pane has been off-screen past the threshold.
  const hiddenTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intersectionObserverRef = useRef<IntersectionObserver | null>(null);

  const { config } = useConfig();
  const termCfg = config.terminal;
  const { terminalTheme } = useTheme();

  // Current config/theme readable from the stable build callback below without
  // forcing it to re-create on every config change.
  const termCfgRef = useRef(termCfg);
  termCfgRef.current = termCfg;
  const terminalThemeRef = useRef(terminalTheme);
  terminalThemeRef.current = terminalTheme;

  const handleExit = useCallback(() => {
    if (terminalRef.current) {
      terminalRef.current.write('\r\n\x1b[90m[Process exited]\x1b[0m\r\n');
    }
  }, []);

  const { sessionId, isReady, write, resize, attachToTerminal, startPTY } = usePTY({
    paneId,
    shell: shell || termCfg.shell,
    cwd,
    onExit: handleExit,
    defer: true, // Don't create PTY until terminal is open and fitted
  });

  // Notify parent of PTY session ID for session save (CWD lookup)
  useEffect(() => {
    if (sessionId && onPtyReady) {
      onPtyReady(paneId, sessionId);
    }
  }, [sessionId, paneId, onPtyReady]);

  // Build a fresh xterm.js instance into the (still-mounted) container, wire up
  // all listeners, and attach it to the live PTY. Returns a cleanup function
  // that tears down THIS instance (listeners + observer + term.dispose) without
  // touching the PTY. Used both for the initial build and — when
  // DISPOSE_HIDDEN_TERMINALS is on — for re-creating after an off-screen
  // disposal. `isFirstBuild` is true only for the very first build, which is the
  // one that starts the PTY; re-creations re-attach to the already-live PTY.
  const buildTerminal = useCallback(
    (container: HTMLDivElement, isFirstBuild: boolean): (() => void) => {
      const termCfgNow = termCfgRef.current;

      const term = new Terminal({
        cursorBlink: termCfgNow.cursorBlink,
        fontSize: termCfgNow.fontSize,
        fontFamily: quoteFontFamily(termCfgNow.fontFamily),
        theme: terminalThemeRef.current,
        allowProposedApi: true,
        scrollback: termCfgNow.scrollback,
        convertEol: false,
        cursorStyle: termCfgNow.cursorStyle as 'block' | 'underline' | 'bar',
        drawBoldTextInBrightColors: true,
      });

      const fitAddon = new FitAddon();
      term.loadAddon(fitAddon);

      terminalRef.current = term;
      fitAddonRef.current = fitAddon;

      // Use web-fonts addon to ensure @font-face fonts are loaded before canvas renders
      const webFontsAddon = new WebFontsAddon();
      term.loadAddon(webFontsAddon);

      webFontsAddon.loadFonts().then(() => {
        term.open(container);
        try {
          fitAddon.fit();
        } catch {}
        if (isFirstBuild && !ptyStartedRef.current) {
          // NOW create the PTY with the correct dimensions
          ptyStartedRef.current = true;
          startPTY(term.cols, term.rows);
        } else {
          // Re-attaching to an already-live PTY after an off-screen disposal.
          // Push the current geometry so the daemon reflows correctly. The
          // daemon replays its output ring buffer to a NEW byte-stream
          // subscriber — but usePTY does not currently re-subscribe on
          // attachToTerminal, so prior contents only return once usePTY grows a
          // resubscribe path (see DISPOSE_HIDDEN_TERMINALS note above).
          resize(term.cols, term.rows);
        }
        // The synchronous term.focus() below runs before the terminal is opened
        // into the DOM, so it's a no-op for a freshly-created pane (e.g. from the
        // command palette). Re-assert focus here now that it's actually attached.
        if (isActiveRef.current) requestAnimationFrame(() => term.focus());
      });

      // Tell xterm to NOT process keys that the app handles.
      // Return false = xterm ignores the key, letting our window capture handler take it.
      term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
        // Ctrl+Shift+C — copy from terminal
        if (e.ctrlKey && e.shiftKey && e.key === 'C') {
          e.preventDefault();
          const sel = term.getSelection();
          if (sel) navigator.clipboard.writeText(sel);
          return false;
        }
        // Ctrl+Shift+V — paste. Let xterm's native paste event deliver the text
        // (single insert, bracketed-paste aware); return false only to suppress ^V.
        if (e.ctrlKey && e.shiftKey && e.key === 'V') {
          return false;
        }
        // Ctrl+C — copy if there's a selection, otherwise let xterm send SIGINT
        if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'c') {
          const sel = term.getSelection();
          if (sel) {
            e.preventDefault();
            navigator.clipboard.writeText(sel);
            term.clearSelection();
            return false;
          }
          return true; // no selection — let xterm send ^C
        }
        // Ctrl+V — paste. Handled by xterm's native paste event (single insert,
        // bracketed-paste aware); return false so xterm doesn't also emit ^V.
        // Manual clipboard.readText + write here caused a double paste, because
        // preventDefault on keydown does not stop the browser's native paste event.
        if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'v') {
          return false;
        }
        // Ctrl+T/B/W/D, Ctrl+/, Ctrl+,, Ctrl+S, Ctrl+K, Ctrl+` (toggle terminal) — app-level
        if (
          e.ctrlKey &&
          !e.altKey &&
          !e.shiftKey &&
          ['t', 'b', 'w', 'd', '/', '?', ',', 's', 'k', '`'].includes(e.key)
        ) {
          return false;
        }
        // Ctrl+1-9 — jump to pane
        if (e.ctrlKey && !e.altKey && !e.shiftKey && /^[1-9]$/.test(e.key)) {
          return false;
        }
        // Alt+Arrow — sub-pane navigation
        if (
          e.altKey &&
          !e.ctrlKey &&
          ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)
        ) {
          return false;
        }
        // Ctrl+Alt+Arrow — tab navigation
        if (e.ctrlKey && e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
          return false;
        }
        // Ctrl+Shift combos (other than C/V above) — resize/move pane
        if (e.ctrlKey && e.shiftKey) {
          return false;
        }
        // F2 — rename
        if (e.key === 'F2') {
          return false;
        }
        // Let xterm handle everything else
        return true;
      });

      // Fit multiple times during startup — the container needs time to reach
      // final size. Guard on the container so a pane created under a hidden agent
      // doesn't fit a zero-size box.
      fitWithRetry(fitAddon, container);

      // Point the PTY output stream at this xterm instance. On a re-build this
      // repoints usePTY's term ref so live bytes flow to the new instance.
      attachToTerminal(term);

      const onDataDisposable = term.onData((data) => {
        write(data);
      });

      const onBinaryDisposable = term.onBinary((data) => {
        write(data);
      });

      const observer = new ResizeObserver(() => {
        // Skip while hidden: toggling a workspace to display:none fires a 0×0
        // resize, and fitting that collapses the grid and garbles the PTY on show.
        if (!isTermVisible(container)) return;
        requestAnimationFrame(() => {
          try {
            if (fitAddonRef.current) {
              fitAddonRef.current.fit();
            }
          } catch {
            // Ignore fit errors during resize
          }
        });
      });
      observer.observe(container);
      resizeObserverRef.current = observer;

      const onResizeDisposable = term.onResize(({ cols, rows }) => {
        resize(cols, rows);
      });

      term.focus();

      // Per-instance cleanup: tears down THIS xterm only. Does NOT touch the PTY.
      return () => {
        onDataDisposable.dispose();
        onBinaryDisposable.dispose();
        onResizeDisposable.dispose();

        if (resizeObserverRef.current) {
          resizeObserverRef.current.disconnect();
          resizeObserverRef.current = null;
        }

        term.dispose();
        terminalRef.current = null;
        fitAddonRef.current = null;
      };
      // termCfg/theme are read via refs so this callback stays stable across
      // config changes; the deps are the stable usePTY functions.
      // eslint-disable-next-line react-hooks/exhaustive-deps
    },
    [attachToTerminal, write, resize, startPTY],
  );

  // Dispose the live xterm instance (canvas + scrollback) while keeping the DOM
  // container and the PTY alive. Safe to call when already disposed.
  const disposeTerminal = useCallback(() => {
    if (xtermDisposedRef.current) return;
    if (instanceCleanupRef.current) {
      instanceCleanupRef.current();
      instanceCleanupRef.current = null;
    }
    xtermDisposedRef.current = true;
  }, []);

  // Re-create the xterm instance into the still-mounted container and re-attach
  // to the live PTY. Safe to call when not currently disposed (no-op).
  const recreateTerminal = useCallback(() => {
    if (!xtermDisposedRef.current) return;
    const container = containerRef.current;
    if (!container) return;
    xtermDisposedRef.current = false;
    instanceCleanupRef.current = buildTerminal(container, /* isFirstBuild */ false);
  }, [buildTerminal]);

  // Initialize xterm.js terminal (first build — starts the PTY).
  useEffect(() => {
    const container = containerRef.current;
    if (!container || initializedRef.current) return;
    initializedRef.current = true;

    instanceCleanupRef.current = buildTerminal(container, /* isFirstBuild */ true);

    return () => {
      // Full unmount: tear down the live instance (if any) and reset flags so a
      // remount rebuilds cleanly. The PTY teardown is owned by usePTY's own
      // unmount cleanup — we do NOT CloseTerminal here.
      if (instanceCleanupRef.current) {
        instanceCleanupRef.current();
        instanceCleanupRef.current = null;
      }
      xtermDisposedRef.current = false;
      initializedRef.current = false;
    };
  }, [buildTerminal]);

  // ── Off-screen disposal lifecycle (gated behind DISPOSE_HIDDEN_TERMINALS) ──
  // Watch the pane's own container with an IntersectionObserver. When it has
  // been off-screen continuously for longer than DISPOSE_HIDDEN_TERMINALS_MS,
  // dispose the xterm instance to free the canvas + scrollback memory. When it
  // returns to view, immediately re-create it and re-attach to the live PTY.
  // The DOM node and the backend PTY stay alive throughout.
  useEffect(() => {
    if (!DISPOSE_HIDDEN_TERMINALS) return;
    const container = containerRef.current;
    if (!container || typeof IntersectionObserver === 'undefined') return;

    const clearHiddenTimer = () => {
      if (hiddenTimerRef.current) {
        clearTimeout(hiddenTimerRef.current);
        hiddenTimerRef.current = null;
      }
    };

    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries[entries.length - 1];
        if (!entry) return;
        const onScreen = entry.isIntersecting && entry.intersectionRatio > 0;
        if (onScreen) {
          // Back in view: cancel any pending disposal and rebuild if needed.
          clearHiddenTimer();
          if (xtermDisposedRef.current) recreateTerminal();
        } else {
          // Off-screen: arm the disposal timer (idempotent — don't re-arm if
          // already pending or already disposed).
          if (xtermDisposedRef.current || hiddenTimerRef.current) return;
          hiddenTimerRef.current = setTimeout(() => {
            hiddenTimerRef.current = null;
            disposeTerminal();
          }, DISPOSE_HIDDEN_TERMINALS_MS);
        }
      },
      { threshold: 0 },
    );

    observer.observe(container);
    intersectionObserverRef.current = observer;

    return () => {
      clearHiddenTimer();
      observer.disconnect();
      intersectionObserverRef.current = null;
    };
  }, [disposeTerminal, recreateTerminal]);

  // Focus/blur terminal when pane becomes active/inactive + re-fit on focus
  useEffect(() => {
    const term = terminalRef.current;
    if (!term) return;
    if (isActive) {
      term.focus();
      // Re-fit + repaint when pane becomes active — the workspace was likely
      // just toggled back from display:none, leaving stale glyphs. Two frames
      // so layout settles before fitting.
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          refitAndRepaint(fitAddonRef.current, term, containerRef.current);
          resize(term.cols, term.rows);
        }),
      );
    } else {
      term.blur();
    }
  }, [isActive, resize]);

  // When PTY becomes ready, do an initial fit + resize sync
  useEffect(() => {
    if (!isReady || !fitAddonRef.current || !terminalRef.current) return;

    const timer = setTimeout(() => {
      const term = terminalRef.current;
      if (!term || !isTermVisible(containerRef.current)) return;
      try {
        fitAddonRef.current?.fit();
        resize(term.cols, term.rows);
      } catch {
        // Ignore
      }
    }, 50);

    return () => clearTimeout(timer);
  }, [isReady, resize]);

  // Run the initial command (script buttons) once the PTY is ready. Small delay
  // so the shell has printed its prompt before we type — otherwise the first
  // keystrokes can be swallowed by shell startup.
  useEffect(() => {
    if (!isReady || !initialCommand || ranInitialRef.current) return;
    ranInitialRef.current = true;
    const timer = setTimeout(() => write(initialCommand + '\r'), 400);
    return () => clearTimeout(timer);
  }, [isReady, initialCommand, write]);

  // Update terminal theme when it changes
  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.options.theme = terminalTheme;
    }
  }, [terminalTheme]);

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', overflow: 'hidden' }}>
      <div
        ref={containerRef}
        style={{
          width: '100%',
          height: '100%',
          overflow: 'hidden',
          backgroundColor: 'var(--wks-bg-terminal)',
        }}
      />
    </div>
  );
};

export default TerminalPane;
