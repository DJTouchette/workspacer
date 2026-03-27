import { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebFontsAddon } from '@xterm/addon-web-fonts';
import '@xterm/xterm/css/xterm.css';
import { usePTY } from '../hooks/usePTY';
import { useConfig, Config } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { registerIssueLinkProvider, type IssuePeekData } from '../lib/issueLinks';
import { claudeColors as colors, ensureKeyframes } from '../components/claude-shared';

/** Ensure each CSS font-family name with spaces is quoted */
function quoteFontFamily(ff: string): string {
  return ff.split(',').map(f => {
    f = f.trim();
    if (!f) return f;
    // Already quoted or a generic family (monospace, serif, etc.)
    if (/^["']/.test(f) || /^(monospace|sans-serif|serif|cursive|fantasy|system-ui)$/i.test(f)) return f;
    // Has spaces — needs quoting
    if (f.includes(' ')) return `"${f}"`;
    return f;
  }).join(', ');
}

interface TerminalPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  shell?: string;
  cwd?: string;
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
}

const TerminalPane: React.FC<TerminalPaneProps> = ({ paneId, title, isActive, shell, cwd, onPtyReady }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const initializedRef = useRef(false);
  const [issuePeek, setIssuePeek] = useState<IssuePeekData | null>(null);

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

    // Issue key link provider — makes PROJ-123 clickable
    const issueLinkDisp = registerIssueLinkProvider(term, (data) => {
      setIssuePeek(data);
    });

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
      issueLinkDisp.dispose();
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
      {issuePeek && (
        <div
          onClick={() => setIssuePeek(null)}
          style={{
            position: 'absolute', inset: 0, zIndex: 100,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            backgroundColor: 'rgba(0,0,0,0.4)',
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              width: 420, maxHeight: '70vh', overflow: 'auto',
              borderRadius: 10, border: `1px solid ${colors.border}`,
              backgroundColor: 'var(--wks-bg-surface)', padding: '16px 20px',
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
              <span style={{ color: colors.accent, fontWeight: 700, fontFamily: 'monospace', fontSize: '0.82rem' }}>
                {issuePeek.key}
              </span>
              <span style={{
                fontSize: '0.6rem', fontWeight: 600, padding: '2px 8px', borderRadius: 10,
                backgroundColor: 'rgba(255,255,255,0.05)',
                color: issuePeek.statusCategory === 'done' ? colors.success : issuePeek.statusCategory === 'in_progress' ? colors.accent : colors.muted,
              }}>
                {issuePeek.status}
              </span>
              <div style={{ flex: 1 }} />
              <span onClick={() => setIssuePeek(null)} style={{ cursor: 'pointer', color: colors.muted, fontSize: '0.85rem' }}>{'\u00D7'}</span>
            </div>
            <div style={{ fontSize: '0.82rem', fontWeight: 600, color: colors.textBright, marginBottom: 8 }}>
              {issuePeek.title}
            </div>
            <div style={{ fontSize: '0.65rem', color: colors.muted, marginBottom: 10, display: 'flex', gap: 12 }}>
              <span>Type: {issuePeek.type}</span>
              {issuePeek.assignee && <span>Assignee: {issuePeek.assignee}</span>}
            </div>
            {issuePeek.description && (
              <div style={{
                fontSize: '0.72rem', lineHeight: 1.5, color: colors.text,
                whiteSpace: 'pre-wrap', padding: '10px 12px', borderRadius: 6,
                border: `1px solid ${colors.borderSubtle}`, backgroundColor: 'rgba(255,255,255,0.02)',
                maxHeight: 200, overflow: 'auto',
              }}>
                {issuePeek.description}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default TerminalPane;
