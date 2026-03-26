import React, { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { usePTY } from '../hooks/usePTY';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { useConfig } from '../hooks/useConfig';
import type { ClaudeSessionSnapshot, ToolCall, ConversationTurn, FileChange, PendingApproval, SubagentInfo } from '../types/claudeSession';

interface ClaudePaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  cwd?: string;
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
}

type ViewMode = 'gui' | 'terminal';

// ── Sub-components ──

const StatusBadge: React.FC<{ session: ClaudeSessionSnapshot | null }> = ({ session }) => {
  if (!session) return <span style={badgeStyle('#555')}>no session</span>;

  const colors: Record<string, string> = {
    idle: '#4ade80',
    thinking: '#facc15',
    streaming: '#60a5fa',
    waiting_input: '#c084fc',
    waiting_approval: '#f87171',
  };

  const labels: Record<string, string> = {
    idle: 'Idle',
    thinking: 'Thinking...',
    streaming: 'Streaming',
    waiting_input: 'Waiting for input',
    waiting_approval: 'Needs approval',
  };

  const color = colors[session.ambientState] ?? '#555';
  const label = labels[session.ambientState] ?? session.ambientState;

  return (
    <span style={badgeStyle(color)}>
      <span style={{ display: 'inline-block', width: 6, height: 6, borderRadius: '50%', backgroundColor: color, marginRight: 4 }} />
      {label}
    </span>
  );
};

function badgeStyle(color: string): React.CSSProperties {
  return {
    display: 'inline-flex',
    alignItems: 'center',
    fontSize: '0.6rem',
    fontWeight: 600,
    color: color,
    padding: '1px 6px',
    borderRadius: 3,
    backgroundColor: 'rgba(255,255,255,0.05)',
    whiteSpace: 'nowrap',
  };
}

const ToolCallItem: React.FC<{ tc: ToolCall }> = ({ tc }) => {
  const [expanded, setExpanded] = useState(false);
  const statusIcon = tc.status === 'running' ? '\u23F3' : tc.status === 'complete' ? '\u2705' : '\u274C';
  const duration = tc.completedAt ? `${((tc.completedAt - tc.startedAt) / 1000).toFixed(1)}s` : 'running';

  return (
    <div style={{ marginBottom: 4, fontSize: '0.65rem' }}>
      <div
        style={{ cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 4, color: 'rgb(180, 190, 220)' }}
        onClick={() => setExpanded(!expanded)}
      >
        <span>{statusIcon}</span>
        <span style={{ fontWeight: 600, color: 'rgb(140, 180, 255)' }}>{tc.name}</span>
        <span style={{ color: 'rgb(120, 120, 140)' }}>{duration}</span>
        {tc.name === 'Edit' || tc.name === 'Write' || tc.name === 'MultiEdit' ? (
          <span style={{ color: 'rgb(100, 100, 120)', fontFamily: 'monospace' }}>
            {tc.input?.file_path?.split('/').pop() ?? ''}
          </span>
        ) : tc.name === 'Bash' ? (
          <span style={{ color: 'rgb(100, 100, 120)', fontFamily: 'monospace' }}>
            {(tc.input?.command ?? '').slice(0, 60)}
          </span>
        ) : null}
        <span style={{ color: 'rgb(80, 80, 100)', marginLeft: 'auto' }}>{expanded ? '\u25B2' : '\u25BC'}</span>
      </div>
      {expanded && (
        <pre style={{
          margin: '2px 0 0 16px',
          padding: 6,
          backgroundColor: 'rgb(18, 18, 22)',
          borderRadius: 4,
          fontSize: '0.6rem',
          color: 'rgb(160, 160, 180)',
          overflowX: 'auto',
          maxHeight: 200,
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-word',
        }}>
          {JSON.stringify(tc.input, null, 2)}
          {tc.response && (
            <>
              {'\n--- response ---\n'}
              {typeof tc.response === 'string' ? tc.response : JSON.stringify(tc.response, null, 2)}
            </>
          )}
        </pre>
      )}
    </div>
  );
};

const ConversationMessage: React.FC<{ turn: ConversationTurn }> = ({ turn }) => {
  const isUser = turn.role === 'user';
  return (
    <div style={{
      marginBottom: 8,
      padding: '6px 10px',
      borderRadius: 6,
      backgroundColor: isUser ? 'rgba(80, 120, 200, 0.12)' : 'rgba(255, 255, 255, 0.03)',
      borderLeft: isUser ? '2px solid rgb(80, 120, 200)' : '2px solid rgb(60, 60, 80)',
    }}>
      <div style={{ fontSize: '0.55rem', color: isUser ? 'rgb(120, 160, 240)' : 'rgb(100, 100, 120)', marginBottom: 2, fontWeight: 600 }}>
        {isUser ? 'You' : 'Claude'}
        <span style={{ marginLeft: 8, fontWeight: 400, color: 'rgb(80, 80, 100)' }}>
          {new Date(turn.timestamp).toLocaleTimeString()}
        </span>
      </div>
      <div style={{ fontSize: '0.65rem', color: 'rgb(200, 200, 220)', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
        {turn.content || '(empty)'}
      </div>
    </div>
  );
};

const FileChangeItem: React.FC<{ fc: FileChange }> = ({ fc }) => {
  const filename = fc.path.split('/').pop() ?? fc.path;
  const dir = fc.path.split('/').slice(0, -1).join('/');
  return (
    <div style={{ fontSize: '0.6rem', padding: '2px 0', display: 'flex', gap: 6, alignItems: 'center' }}>
      <span style={{ color: fc.toolName === 'Write' ? 'rgb(74, 222, 128)' : 'rgb(250, 204, 21)', fontWeight: 600 }}>
        {fc.toolName === 'Write' ? '+' : '~'}
      </span>
      <span style={{ color: 'rgb(180, 190, 220)', fontFamily: 'monospace' }}>{filename}</span>
      <span style={{ color: 'rgb(80, 80, 100)', fontFamily: 'monospace' }}>{dir}</span>
    </div>
  );
};

const ApprovalPrompt: React.FC<{ approval: PendingApproval; onRespond: (response: string) => void }> = ({ approval, onRespond }) => (
  <div style={{
    padding: 10,
    margin: '8px 0',
    borderRadius: 6,
    backgroundColor: 'rgba(248, 113, 113, 0.1)',
    border: '1px solid rgba(248, 113, 113, 0.3)',
  }}>
    <div style={{ fontSize: '0.65rem', color: 'rgb(248, 113, 113)', fontWeight: 600, marginBottom: 4 }}>
      Permission Required: {approval.toolName}
    </div>
    <pre style={{
      fontSize: '0.6rem',
      color: 'rgb(180, 180, 200)',
      margin: '4px 0',
      padding: 6,
      backgroundColor: 'rgb(18, 18, 22)',
      borderRadius: 4,
      maxHeight: 120,
      overflow: 'auto',
      whiteSpace: 'pre-wrap',
    }}>
      {JSON.stringify(approval.toolInput, null, 2)}
    </pre>
    <div style={{ display: 'flex', gap: 6, marginTop: 6 }}>
      <button style={approvalBtnStyle('rgb(74, 222, 128)')} onClick={() => onRespond('y')}>Allow</button>
      <button style={approvalBtnStyle('rgb(248, 113, 113)')} onClick={() => onRespond('n')}>Deny</button>
    </div>
  </div>
);

function approvalBtnStyle(color: string): React.CSSProperties {
  return {
    fontSize: '0.6rem',
    fontWeight: 600,
    padding: '3px 12px',
    borderRadius: 4,
    border: `1px solid ${color}`,
    backgroundColor: 'transparent',
    color,
    cursor: 'pointer',
  };
}

const SubagentItem: React.FC<{ sub: SubagentInfo }> = ({ sub }) => (
  <div style={{ fontSize: '0.6rem', color: 'rgb(160, 160, 180)', display: 'flex', gap: 4, alignItems: 'center', padding: '1px 0' }}>
    <span>{sub.status === 'running' ? '\u23F3' : '\u2705'}</span>
    <span style={{ fontWeight: 600, color: 'rgb(192, 132, 252)' }}>{sub.type}</span>
    <span style={{ color: 'rgb(80, 80, 100)' }}>{sub.id.slice(0, 8)}</span>
  </div>
);

// ── Main component ──

const TERMINAL_THEME = {
  background: '#121214',
  foreground: '#e4e4e7',
  cursor: '#e4e4e7',
  cursorAccent: '#121214',
  selectionBackground: 'rgba(128, 160, 255, 0.3)',
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

const ClaudePane: React.FC<ClaudePaneProps> = ({ paneId, title, isActive, cwd, onPtyReady }) => {
  const [viewMode, setViewMode] = useState<ViewMode>('gui');
  const termContainerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const termInitRef = useRef(false);
  const conversationEndRef = useRef<HTMLDivElement>(null);

  const { config } = useConfig();
  const termCfg = config.terminal;

  const handleExit = useCallback(() => {
    if (terminalRef.current) {
      terminalRef.current.write('\r\n\x1b[90m[Claude session exited]\x1b[0m\r\n');
    }
  }, []);

  // Use a special 'claude' shell marker — the PTY hook in main process handles this
  const { sessionId, isReady, write, resize, attachToTerminal } = usePTY({
    paneId,
    shell: '__claude__', // sentinel value intercepted by createTerminal
    cwd,
    onExit: handleExit,
  });

  const { session } = useClaudeSession({ ptySessionId: sessionId });

  // Notify parent of PTY session ID
  useEffect(() => {
    if (sessionId && onPtyReady) {
      onPtyReady(paneId, sessionId);
    }
  }, [sessionId, paneId, onPtyReady]);

  // Initialize xterm.js for the terminal view (hidden when in GUI mode)
  useEffect(() => {
    const container = termContainerRef.current;
    if (!container || termInitRef.current) return;
    termInitRef.current = true;

    const term = new Terminal({
      cursorBlink: termCfg.cursorBlink,
      fontSize: termCfg.fontSize,
      fontFamily: termCfg.fontFamily,
      theme: TERMINAL_THEME,
      allowProposedApi: true,
      scrollback: termCfg.scrollback,
      convertEol: false,
      cursorStyle: termCfg.cursorStyle as 'block' | 'underline' | 'bar',
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    term.open(container);

    // Forward app-level keys
    term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      if (e.ctrlKey && !e.altKey && ['t', 'b', 'w', 'd', '/', '?', ',', 's', 'k'].includes(e.key)) return false;
      if (e.ctrlKey && !e.altKey && !e.shiftKey && /^[1-9]$/.test(e.key)) return false;
      if (e.altKey && !e.ctrlKey && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return false;
      if (e.ctrlKey && e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) return false;
      if (e.ctrlKey && e.shiftKey) return false;
      if (e.key === 'F2') return false;
      return true;
    });

    const fitRetry = () => { try { fitAddon.fit(); } catch {} };
    requestAnimationFrame(fitRetry);
    setTimeout(fitRetry, 100);
    setTimeout(fitRetry, 300);

    attachToTerminal(term);

    const onDataDisp = term.onData((data) => write(data));
    const onBinaryDisp = term.onBinary((data) => write(data));

    const observer = new ResizeObserver(() => {
      requestAnimationFrame(() => { try { fitAddonRef.current?.fit(); } catch {} });
    });
    observer.observe(container);

    const onResizeDisp = term.onResize(({ cols, rows }) => resize(cols, rows));

    return () => {
      onDataDisp.dispose();
      onBinaryDisp.dispose();
      onResizeDisp.dispose();
      observer.disconnect();
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      termInitRef.current = false;
    };
  }, [attachToTerminal, write, resize]);

  // Focus terminal when switching to terminal mode
  useEffect(() => {
    if (viewMode === 'terminal' && isActive && terminalRef.current) {
      terminalRef.current.focus();
      requestAnimationFrame(() => { try { fitAddonRef.current?.fit(); } catch {} });
    }
  }, [viewMode, isActive]);

  // Re-fit terminal on active change
  useEffect(() => {
    if (isActive && viewMode === 'terminal' && terminalRef.current) {
      terminalRef.current.focus();
      requestAnimationFrame(() => {
        try {
          fitAddonRef.current?.fit();
          const t = terminalRef.current;
          if (t) resize(t.cols, t.rows);
        } catch {}
      });
    }
  }, [isActive, viewMode, resize]);

  // Auto-scroll conversation to bottom
  useEffect(() => {
    conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [session?.conversation?.length, session?.activeToolCalls?.length]);

  // Send approval response by typing into the PTY
  const handleApprovalRespond = useCallback((response: string) => {
    write(response + '\n');
  }, [write]);

  // ── Render ──

  const activeToolCalls = session?.activeToolCalls ?? [];
  const completedToolCalls = session?.completedToolCalls ?? [];
  const conversation = session?.conversation ?? [];
  const fileChanges = session?.fileChanges ?? [];
  const subagents = session?.subagents ?? [];
  const pendingApproval = session?.pendingApproval ?? null;

  return (
    <div style={{
      width: '100%',
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      backgroundColor: '#121214',
      color: 'rgb(200, 200, 220)',
    }}>
      {/* Toolbar */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '3px 8px',
        backgroundColor: 'rgb(24, 24, 30)',
        borderBottom: '1px solid rgb(40, 40, 50)',
        minHeight: 24,
      }}>
        <StatusBadge session={session} />

        {session && (
          <span style={{ fontSize: '0.55rem', color: 'rgb(80, 80, 100)' }}>
            {session.totalToolCalls} tools
          </span>
        )}

        {subagents.filter(s => s.status === 'running').length > 0 && (
          <span style={{ fontSize: '0.55rem', color: 'rgb(192, 132, 252)' }}>
            {subagents.filter(s => s.status === 'running').length} subagent(s)
          </span>
        )}

        <div style={{ flex: 1 }} />

        {/* View mode toggle */}
        <div style={{ display: 'flex', gap: 2 }}>
          <button
            onClick={() => setViewMode('gui')}
            style={{
              ...toggleBtnStyle,
              backgroundColor: viewMode === 'gui' ? 'rgb(60, 70, 100)' : 'transparent',
              color: viewMode === 'gui' ? 'rgb(180, 200, 255)' : 'rgb(100, 100, 120)',
            }}
          >
            GUI
          </button>
          <button
            onClick={() => setViewMode('terminal')}
            style={{
              ...toggleBtnStyle,
              backgroundColor: viewMode === 'terminal' ? 'rgb(60, 70, 100)' : 'transparent',
              color: viewMode === 'terminal' ? 'rgb(180, 200, 255)' : 'rgb(100, 100, 120)',
            }}
          >
            Term
          </button>
        </div>
      </div>

      {/* Content area */}
      <div style={{ flex: 1, overflow: 'hidden', position: 'relative' }}>
        {/* Terminal view (always mounted, visibility toggled) */}
        <div
          ref={termContainerRef}
          style={{
            position: 'absolute',
            inset: 0,
            display: viewMode === 'terminal' ? 'block' : 'none',
          }}
        />

        {/* GUI view */}
        {viewMode === 'gui' && (
          <div style={{
            height: '100%',
            display: 'flex',
            flexDirection: 'row',
            overflow: 'hidden',
          }}>
            {/* Main conversation panel */}
            <div style={{
              flex: 1,
              display: 'flex',
              flexDirection: 'column',
              overflow: 'hidden',
            }}>
              {/* Conversation scroll area */}
              <div style={{
                flex: 1,
                overflowY: 'auto',
                padding: '8px 12px',
              }}>
                {conversation.length === 0 && !session && (
                  <div style={{ textAlign: 'center', marginTop: 40, color: 'rgb(80, 80, 100)' }}>
                    <div style={{ fontSize: '1.5rem', marginBottom: 8 }}>&#9670;</div>
                    <div style={{ fontSize: '0.7rem' }}>Claude Code session starting...</div>
                    <div style={{ fontSize: '0.6rem', marginTop: 4 }}>
                      Waiting for hook events. Make sure hooks are configured in ~/.claude/settings.json
                    </div>
                  </div>
                )}

                {conversation.length === 0 && session && (
                  <div style={{ textAlign: 'center', marginTop: 40, color: 'rgb(80, 80, 100)' }}>
                    <div style={{ fontSize: '1.5rem', marginBottom: 8 }}>&#9670;</div>
                    <div style={{ fontSize: '0.7rem' }}>Session connected</div>
                    <div style={{ fontSize: '0.6rem', marginTop: 4 }}>
                      Waiting for conversation activity...
                    </div>
                  </div>
                )}

                {conversation.map((turn, i) => (
                  <ConversationMessage key={i} turn={turn} />
                ))}

                {/* Active tool calls */}
                {activeToolCalls.length > 0 && (
                  <div style={{ margin: '8px 0', padding: '6px 10px', borderRadius: 6, backgroundColor: 'rgba(96, 165, 250, 0.08)', borderLeft: '2px solid rgb(96, 165, 250)' }}>
                    <div style={{ fontSize: '0.55rem', color: 'rgb(96, 165, 250)', fontWeight: 600, marginBottom: 4 }}>Running</div>
                    {activeToolCalls.map(tc => <ToolCallItem key={tc.id} tc={tc} />)}
                  </div>
                )}

                {/* Pending approval */}
                {pendingApproval && (
                  <ApprovalPrompt approval={pendingApproval} onRespond={handleApprovalRespond} />
                )}

                <div ref={conversationEndRef} />
              </div>

              {/* Input area — type directly into PTY */}
              <div style={{
                borderTop: '1px solid rgb(40, 40, 50)',
                padding: '6px 8px',
                display: 'flex',
                gap: 6,
              }}>
                <input
                  placeholder="Type a message... (sent to Claude terminal)"
                  style={{
                    flex: 1,
                    fontSize: '0.65rem',
                    padding: '4px 8px',
                    borderRadius: 4,
                    border: '1px solid rgb(50, 50, 65)',
                    backgroundColor: 'rgb(18, 18, 22)',
                    color: 'rgb(200, 200, 220)',
                    outline: 'none',
                    fontFamily: 'inherit',
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && !e.shiftKey) {
                      e.preventDefault();
                      const val = (e.target as HTMLInputElement).value;
                      if (val.trim()) {
                        write(val + '\n');
                        (e.target as HTMLInputElement).value = '';
                      }
                    }
                  }}
                />
              </div>
            </div>

            {/* Side panel: tool calls + file changes */}
            <div style={{
              width: 220,
              minWidth: 220,
              borderLeft: '1px solid rgb(40, 40, 50)',
              overflowY: 'auto',
              padding: '6px 8px',
              backgroundColor: 'rgb(16, 16, 20)',
            }}>
              {/* File changes */}
              {fileChanges.length > 0 && (
                <div style={{ marginBottom: 12 }}>
                  <div style={{ fontSize: '0.55rem', color: 'rgb(100, 100, 120)', fontWeight: 600, marginBottom: 4, textTransform: 'uppercase', letterSpacing: 0.5 }}>
                    Files ({fileChanges.length})
                  </div>
                  {fileChanges.slice(-20).map((fc, i) => <FileChangeItem key={i} fc={fc} />)}
                </div>
              )}

              {/* Subagents */}
              {subagents.length > 0 && (
                <div style={{ marginBottom: 12 }}>
                  <div style={{ fontSize: '0.55rem', color: 'rgb(100, 100, 120)', fontWeight: 600, marginBottom: 4, textTransform: 'uppercase', letterSpacing: 0.5 }}>
                    Subagents ({subagents.length})
                  </div>
                  {subagents.map(sub => <SubagentItem key={sub.id} sub={sub} />)}
                </div>
              )}

              {/* Recent completed tool calls */}
              {completedToolCalls.length > 0 && (
                <div>
                  <div style={{ fontSize: '0.55rem', color: 'rgb(100, 100, 120)', fontWeight: 600, marginBottom: 4, textTransform: 'uppercase', letterSpacing: 0.5 }}>
                    Tool History ({completedToolCalls.length})
                  </div>
                  {completedToolCalls.slice(-30).map(tc => <ToolCallItem key={tc.id} tc={tc} />)}
                </div>
              )}

              {completedToolCalls.length === 0 && fileChanges.length === 0 && subagents.length === 0 && (
                <div style={{ textAlign: 'center', marginTop: 20, fontSize: '0.6rem', color: 'rgb(60, 60, 80)' }}>
                  No activity yet
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

const toggleBtnStyle: React.CSSProperties = {
  fontSize: '0.55rem',
  fontWeight: 600,
  padding: '2px 8px',
  borderRadius: 3,
  border: 'none',
  cursor: 'pointer',
};

export default ClaudePane;
