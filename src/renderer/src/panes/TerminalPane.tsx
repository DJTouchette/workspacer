import { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebFontsAddon } from '@xterm/addon-web-fonts';
import '@xterm/xterm/css/xterm.css';
import { usePTY } from '../hooks/usePTY';
import { useConfig, Config } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { claudeColors as colors, ensureKeyframes } from '../components/claude-shared';
import { quoteFontFamily, fitWithRetry } from '../lib/terminalUtils';

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

const TerminalPane: React.FC<TerminalPaneProps> = ({ paneId, title, isActive, shell, cwd, initialCommand, onPtyReady }) => {
  const ranInitialRef = useRef(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const initializedRef = useRef(false);
  // Latest active state, readable from async init callbacks below.
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;

  const { config } = useConfig();
  const termCfg = config.terminal;
  const { terminalTheme } = useTheme();

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

  // Initialize xterm.js terminal
  useEffect(() => {
    const container = containerRef.current;
    if (!container || initializedRef.current) return;
    initializedRef.current = true;

    const term = new Terminal({
      cursorBlink: termCfg.cursorBlink,
      fontSize: termCfg.fontSize,
      fontFamily: quoteFontFamily(termCfg.fontFamily),
      theme: terminalTheme,
      allowProposedApi: true,
      scrollback: termCfg.scrollback,
      convertEol: false,
      cursorStyle: termCfg.cursorStyle as 'block' | 'underline' | 'bar',
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
      try { fitAddon.fit(); } catch {}
      // NOW create the PTY with the correct dimensions
      startPTY(term.cols, term.rows);
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
      if (e.ctrlKey && !e.altKey && !e.shiftKey && ['t', 'b', 'w', 'd', '/', '?', ',', 's', 'k', '`'].includes(e.key)) {
        return false;
      }
      // Ctrl+1-9 — jump to pane
      if (e.ctrlKey && !e.altKey && !e.shiftKey && /^[1-9]$/.test(e.key)) {
        return false;
      }
      // Alt+Arrow — sub-pane navigation
      if (e.altKey && !e.ctrlKey && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) {
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

    // Fit multiple times during startup — the container needs time to reach final size
    fitWithRetry(fitAddon);

    attachToTerminal(term);

    const onDataDisposable = term.onData((data) => {
      write(data);
    });

    const onBinaryDisposable = term.onBinary((data) => {
      write(data);
    });

    const observer = new ResizeObserver(() => {
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
      initializedRef.current = false;
    };
  }, [attachToTerminal, write, resize]);

  // Focus/blur terminal when pane becomes active/inactive + re-fit on focus
  useEffect(() => {
    const term = terminalRef.current;
    if (!term) return;
    if (isActive) {
      term.focus();
      // Re-fit when pane becomes active — size may have changed
      requestAnimationFrame(() => {
        try {
          fitAddonRef.current?.fit();
          resize(term.cols, term.rows);
        } catch {
          // Ignore
        }
      });
    } else {
      term.blur();
    }
  }, [isActive, resize]);

  // When PTY becomes ready, do an initial fit + resize sync
  useEffect(() => {
    if (!isReady || !fitAddonRef.current || !terminalRef.current) return;

    const timer = setTimeout(() => {
      try {
        fitAddonRef.current?.fit();
        const term = terminalRef.current;
        if (term) {
          resize(term.cols, term.rows);
        }
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
