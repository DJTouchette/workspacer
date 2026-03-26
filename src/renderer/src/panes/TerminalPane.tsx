import { useEffect, useRef, useCallback } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { usePTY } from '../hooks/usePTY';
import { useConfig, Config } from '../hooks/useConfig';

interface TerminalPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  shell?: string;
  cwd?: string;
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
}

const TERMINAL_THEME = {
  background: '#121214',
  foreground: '#e4e4e7',
  cursor: '#e4e4e7',
  cursorAccent: '#121214',
  selectionBackground: 'rgba(128, 160, 255, 0.3)',
  selectionForeground: undefined,
  black: '#1e1e21',
  red: '#f87171',
  green: '#4ade80',
  yellow: '#facc15',
  blue: '#60a5fa',
  magenta: '#c084fc',
  cyan: '#22d3ee',
  white: '#e4e4e7',
  brightBlack: '#71717a',
  brightRed: '#fca5a5',
  brightGreen: '#86efac',
  brightYellow: '#fde68a',
  brightBlue: '#93c5fd',
  brightMagenta: '#d8b4fe',
  brightCyan: '#67e8f9',
  brightWhite: '#fafafa',
};

const TerminalPane: React.FC<TerminalPaneProps> = ({ paneId, title, isActive, shell, cwd, onPtyReady }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const initializedRef = useRef(false);

  const { config } = useConfig();
  const termCfg = config.terminal;

  const handleExit = useCallback(() => {
    if (terminalRef.current) {
      terminalRef.current.write('\r\n\x1b[90m[Process exited]\x1b[0m\r\n');
    }
  }, []);

  const { sessionId, isReady, write, resize, attachToTerminal } = usePTY({
    paneId,
    shell: shell || termCfg.shell,
    cwd,
    onExit: handleExit,
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
      fontFamily: termCfg.fontFamily,
      theme: TERMINAL_THEME,
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

    // Wait for Nerd Fonts to load before opening terminal
    // (canvas/WebGL won't re-render glyphs after initial draw)
    const fontsReady = (window as any).__fontsReady ?? Promise.resolve();
    fontsReady.then(() => {
      term.open(container);

      // Use GPU-accelerated WebGL renderer
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => { webgl.dispose(); });
        term.loadAddon(webgl);
      } catch (e) {
        console.warn('[TerminalPane] WebGL init failed, using canvas fallback:', e);
      }

      // Force xterm to re-measure glyphs with the loaded font
      const currentFont = term.options.fontFamily;
      term.options.fontFamily = 'monospace';
      term.options.fontFamily = currentFont;

      const fitRetry = () => { try { fitAddon.fit(); } catch {} };
      requestAnimationFrame(fitRetry);
      setTimeout(fitRetry, 100);
      setTimeout(fitRetry, 300);
    });

    // Tell xterm to NOT process keys that the app handles.
    // Return false = xterm ignores the key, letting our window capture handler take it.
    term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      // Ctrl+T, Ctrl+B, Ctrl+W, Ctrl+/, Ctrl+, — always app-level
      if (e.ctrlKey && !e.altKey && ['t', 'b', 'w', 'd', '/', '?', ',', 's', 'k'].includes(e.key)) {
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
      // Ctrl+Shift combos — resize/move pane
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
    const fitWithRetry = () => {
      try {
        fitAddon.fit();
      } catch {
        // Ignore fit errors
      }
    };
    requestAnimationFrame(fitWithRetry);
    setTimeout(fitWithRetry, 100);
    setTimeout(fitWithRetry, 300);

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

  return (
    <div
      ref={containerRef}
      style={{
        width: '100%',
        height: '100%',
        overflow: 'hidden',
        backgroundColor: '#121214',
      }}
    />
  );
};

export default TerminalPane;
