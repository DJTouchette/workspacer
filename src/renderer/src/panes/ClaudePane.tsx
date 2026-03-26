import React, { useEffect, useRef, useCallback, useState, useMemo } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
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

// ── Color Palette ──

const colors = {
  bg: '#0d0d10',
  bgSecondary: 'rgba(255,255,255,0.03)',
  bgToolbar: 'rgba(13, 13, 16, 0.95)',
  userBubble: 'rgba(60, 90, 160, 0.15)',
  userBubbleBorder: 'rgba(60, 90, 160, 0.25)',
  muted: 'rgb(100, 105, 130)',
  mutedDim: 'rgb(65, 68, 85)',
  accent: 'rgb(96, 165, 250)',
  success: 'rgb(74, 222, 128)',
  error: 'rgb(248, 113, 113)',
  warning: '#facc15',
  text: 'rgb(200, 205, 225)',
  textBright: 'rgb(230, 230, 245)',
  border: 'rgba(255, 255, 255, 0.06)',
  borderSubtle: 'rgba(255, 255, 255, 0.04)',
  divider: 'rgba(255, 255, 255, 0.05)',
};

// ── CSS Keyframes (injected once) ──

const STYLE_ID = 'claude-pane-keyframes';

function ensureKeyframes() {
  if (typeof document === 'undefined') return;
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = `
    @keyframes claudePulseDot {
      0%, 80%, 100% { opacity: 0.2; transform: scale(0.85); }
      40% { opacity: 1; transform: scale(1); }
    }
    @keyframes claudeSpinner {
      to { transform: rotate(360deg); }
    }
    @keyframes claudeFadeIn {
      from { opacity: 0; transform: translateY(4px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @keyframes claudeScrollBtn {
      from { opacity: 0; transform: translateY(8px); }
      to { opacity: 1; transform: translateY(0); }
    }
  `;
  document.head.appendChild(style);
}

// ── Sub-components ──

const StatusBadge: React.FC<{ session: ClaudeSessionSnapshot | null }> = ({ session }) => {
  if (!session) return <span style={badgeStyle('#555')}>no session</span>;

  const badgeColors: Record<string, string> = {
    idle: colors.success,
    thinking: colors.warning,
    streaming: colors.accent,
    waiting_input: '#c084fc',
    waiting_approval: colors.error,
  };

  const labels: Record<string, string> = {
    idle: 'Idle',
    thinking: 'Thinking...',
    streaming: 'Streaming',
    waiting_input: 'Waiting for input',
    waiting_approval: 'Needs approval',
  };

  const color = badgeColors[session.ambientState] ?? '#555';
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

// ── Working Timer ──

const WorkingTimer: React.FC<{ session: ClaudeSessionSnapshot | null }> = ({ session }) => {
  const [elapsed, setElapsed] = useState(0);
  const isWorking = session?.ambientState === 'thinking' || session?.ambientState === 'streaming';

  useEffect(() => {
    if (!isWorking || !session) {
      setElapsed(0);
      return;
    }
    const start = session.lastActivity;
    const tick = () => setElapsed(Math.floor((Date.now() - start) / 1000));
    tick();
    const iv = setInterval(tick, 1000);
    return () => clearInterval(iv);
  }, [isWorking, session?.lastActivity]);

  if (!isWorking) return null;

  const fmt = elapsed < 60 ? `${elapsed}s` : `${Math.floor(elapsed / 60)}m ${elapsed % 60}s`;

  return (
    <span style={{ fontSize: '0.55rem', color: colors.muted, fontVariantNumeric: 'tabular-nums' }}>
      Working... {fmt}
    </span>
  );
};

// ── Streaming Dots ──

const StreamingDots: React.FC = () => (
  <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
    {[0, 200, 400].map((delay) => (
      <span
        key={delay}
        style={{
          display: 'inline-block',
          width: 6,
          height: 6,
          borderRadius: '50%',
          backgroundColor: colors.muted,
          animation: `claudePulseDot 1.4s ease-in-out infinite`,
          animationDelay: `${delay}ms`,
        }}
      />
    ))}
  </div>
);

// ── Inline Work Log (collapsed tool calls) ──

const InlineWorkLog: React.FC<{ toolCalls: ToolCall[] }> = ({ toolCalls }) => {
  const [expanded, setExpanded] = useState(false);

  if (toolCalls.length === 0) return null;

  const runningCount = toolCalls.filter(tc => tc.status === 'running').length;

  return (
    <div style={{
      margin: '4px 0 8px 0',
      borderRadius: 8,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
    }}>
      <div
        onClick={() => setExpanded(!expanded)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '5px 10px',
          cursor: 'pointer',
          fontSize: '0.7rem',
          color: colors.muted,
          userSelect: 'none',
        }}
      >
        <span style={{
          display: 'inline-block',
          width: 10,
          fontSize: '0.55rem',
          transition: 'transform 0.15s',
          transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)',
        }}>
          {'\u25B6'}
        </span>
        <span>{toolCalls.length} tool call{toolCalls.length !== 1 ? 's' : ''}</span>
        {runningCount > 0 && (
          <span style={{
            display: 'inline-block',
            width: 10,
            height: 10,
            border: `1.5px solid ${colors.accent}`,
            borderTopColor: 'transparent',
            borderRadius: '50%',
            animation: 'claudeSpinner 0.8s linear infinite',
          }} />
        )}
      </div>
      {expanded && (
        <div style={{ padding: '0 10px 6px 10px' }}>
          {toolCalls.map(tc => <WorkLogEntry key={tc.id} tc={tc} />)}
        </div>
      )}
    </div>
  );
};

const WorkLogEntry: React.FC<{ tc: ToolCall }> = ({ tc }) => {
  const isRunning = tc.status === 'running';
  const isFailed = tc.status === 'failed';

  const icon = isRunning ? null : isFailed ? '\u2717' : '\u2713';
  const iconColor = isRunning ? colors.accent : isFailed ? colors.error : colors.success;

  let detail = '';
  if (tc.name === 'Edit' || tc.name === 'Write' || tc.name === 'MultiEdit') {
    detail = tc.input?.file_path?.split('/').pop() ?? '';
  } else if (tc.name === 'Bash') {
    detail = (tc.input?.command ?? '').slice(0, 50);
  } else if (tc.name === 'Read') {
    detail = tc.input?.file_path?.split('/').pop() ?? '';
  } else if (tc.name === 'Grep') {
    detail = tc.input?.pattern ?? '';
  }

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      gap: 6,
      padding: '2px 0',
      fontSize: '0.7rem',
    }}>
      {isRunning ? (
        <span style={{
          display: 'inline-block',
          width: 10,
          height: 10,
          border: `1.5px solid ${colors.accent}`,
          borderTopColor: 'transparent',
          borderRadius: '50%',
          animation: 'claudeSpinner 0.8s linear infinite',
          flexShrink: 0,
        }} />
      ) : (
        <span style={{ color: iconColor, fontSize: '0.65rem', width: 10, textAlign: 'center', flexShrink: 0 }}>{icon}</span>
      )}
      <span style={{ color: colors.accent, fontWeight: 600 }}>{tc.name}</span>
      {detail && (
        <span style={{
          color: colors.mutedDim,
          fontFamily: 'monospace',
          fontSize: '0.65rem',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          maxWidth: 300,
        }}>
          {detail}
        </span>
      )}
    </div>
  );
};

// ── Lightweight Markdown Renderer ──

/** Render inline markdown: **bold**, *italic*, `code`, [links](url) */
function renderInlineMarkdown(text: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const regex = /(\*\*(.+?)\*\*|\*(.+?)\*|`([^`]+)`|\[([^\]]+)\]\(([^)]+)\))/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  let key = 0;

  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIndex) {
      nodes.push(text.slice(lastIndex, match.index));
    }

    if (match[2]) {
      nodes.push(<strong key={key++} style={{ color: colors.textBright, fontWeight: 700 }}>{match[2]}</strong>);
    } else if (match[3]) {
      nodes.push(<em key={key++} style={{ color: 'rgb(210, 210, 230)', fontStyle: 'italic' }}>{match[3]}</em>);
    } else if (match[4]) {
      nodes.push(
        <code key={key++} style={{
          backgroundColor: 'rgba(255, 255, 255, 0.07)',
          padding: '1px 5px',
          borderRadius: 3,
          fontSize: '0.9em',
          fontFamily: 'var(--claude-mono-font, monospace)',
          color: 'rgb(180, 210, 255)',
        }}>
          {match[4]}
        </code>
      );
    } else if (match[5] && match[6]) {
      nodes.push(
        <span key={key++} style={{ color: colors.accent, textDecoration: 'underline', cursor: 'default' }} title={match[6]}>
          {match[5]}
        </span>
      );
    }

    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < text.length) {
    nodes.push(text.slice(lastIndex));
  }

  return nodes.length > 0 ? nodes : [text];
}

/** Parse a markdown string into structured blocks */
function parseMarkdownBlocks(text: string): React.ReactNode[] {
  if (!text) return [];

  const lines = text.split('\n');
  const blocks: React.ReactNode[] = [];
  let key = 0;
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block
    if (line.trimStart().startsWith('```')) {
      const lang = line.trimStart().slice(3).trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith('```')) {
        codeLines.push(lines[i]);
        i++;
      }
      i++;
      blocks.push(
        <div key={key++} style={{ margin: '6px 0' }}>
          {lang && (
            <div style={{
              fontSize: '0.6rem',
              color: colors.muted,
              backgroundColor: 'rgba(255,255,255,0.04)',
              padding: '2px 10px',
              borderRadius: '6px 6px 0 0',
              borderBottom: `1px solid ${colors.border}`,
              fontFamily: 'var(--claude-mono-font, monospace)',
            }}>
              {lang}
            </div>
          )}
          <pre style={{
            margin: 0,
            padding: '10px 12px',
            backgroundColor: 'rgba(0, 0, 0, 0.3)',
            borderRadius: lang ? '0 0 6px 6px' : '6px',
            fontSize: '0.75rem',
            lineHeight: 1.5,
            color: 'rgb(190, 200, 220)',
            fontFamily: 'var(--claude-mono-font, monospace)',
            overflowX: 'auto',
            whiteSpace: 'pre',
            border: `1px solid ${colors.border}`,
            borderTop: lang ? 'none' : undefined,
          }}>
            {codeLines.join('\n')}
          </pre>
        </div>
      );
      continue;
    }

    // Heading
    const headingMatch = line.match(/^(#{1,4})\s+(.+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      const sizes: Record<number, string> = { 1: '0.95rem', 2: '0.88rem', 3: '0.82rem', 4: '0.78rem' };
      blocks.push(
        <div key={key++} style={{
          fontSize: sizes[level] ?? '0.78rem',
          fontWeight: 700,
          color: colors.textBright,
          margin: `${level === 1 ? 12 : 8}px 0 4px 0`,
          paddingBottom: level <= 2 ? 4 : 0,
          borderBottom: level <= 2 ? `1px solid ${colors.divider}` : 'none',
        }}>
          {renderInlineMarkdown(headingMatch[2])}
        </div>
      );
      i++;
      continue;
    }

    // Unordered list
    if (/^\s*[-*]\s+/.test(line)) {
      const listItems: { indent: number; content: string }[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) {
        const m = lines[i].match(/^(\s*)[-*]\s+(.+)$/);
        if (m) listItems.push({ indent: m[1].length, content: m[2] });
        i++;
      }
      blocks.push(
        <div key={key++} style={{ margin: '4px 0' }}>
          {listItems.map((item, idx) => (
            <div key={idx} style={{ display: 'flex', gap: 6, paddingLeft: Math.min(item.indent, 12) + 4, marginBottom: 2 }}>
              <span style={{ color: colors.accent, flexShrink: 0, lineHeight: 1.6 }}>{'\u2022'}</span>
              <span style={{ lineHeight: 1.6 }}>{renderInlineMarkdown(item.content)}</span>
            </div>
          ))}
        </div>
      );
      continue;
    }

    // Ordered list
    if (/^\s*\d+[.)]\s+/.test(line)) {
      const listItems: { num: string; content: string }[] = [];
      while (i < lines.length && /^\s*\d+[.)]\s+/.test(lines[i])) {
        const m = lines[i].match(/^\s*(\d+)[.)]\s+(.+)$/);
        if (m) listItems.push({ num: m[1], content: m[2] });
        i++;
      }
      blocks.push(
        <div key={key++} style={{ margin: '4px 0' }}>
          {listItems.map((item, idx) => (
            <div key={idx} style={{ display: 'flex', gap: 6, paddingLeft: 4, marginBottom: 2 }}>
              <span style={{ color: colors.muted, flexShrink: 0, minWidth: 14, textAlign: 'right', lineHeight: 1.6 }}>{item.num}.</span>
              <span style={{ lineHeight: 1.6 }}>{renderInlineMarkdown(item.content)}</span>
            </div>
          ))}
        </div>
      );
      continue;
    }

    // Horizontal rule
    if (/^[-*_]{3,}\s*$/.test(line.trim())) {
      blocks.push(<hr key={key++} style={{ border: 'none', borderTop: `1px solid ${colors.divider}`, margin: '8px 0' }} />);
      i++;
      continue;
    }

    // Blockquote
    if (line.startsWith('>')) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].startsWith('>')) {
        quoteLines.push(lines[i].replace(/^>\s?/, ''));
        i++;
      }
      blocks.push(
        <div key={key++} style={{
          borderLeft: `2px solid ${colors.muted}`,
          paddingLeft: 10,
          margin: '4px 0',
          color: 'rgb(160, 165, 185)',
          fontStyle: 'italic',
          lineHeight: 1.6,
        }}>
          {renderInlineMarkdown(quoteLines.join(' '))}
        </div>
      );
      continue;
    }

    // Empty line
    if (line.trim() === '') {
      blocks.push(<div key={key++} style={{ height: 4 }} />);
      i++;
      continue;
    }

    // Regular paragraph
    const paraLines: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== '' &&
      !lines[i].trimStart().startsWith('```') &&
      !lines[i].match(/^#{1,4}\s+/) &&
      !/^\s*[-*]\s+/.test(lines[i]) &&
      !/^\s*\d+[.)]\s+/.test(lines[i]) &&
      !lines[i].startsWith('>') &&
      !/^[-*_]{3,}\s*$/.test(lines[i].trim())
    ) {
      paraLines.push(lines[i]);
      i++;
    }

    if (paraLines.length > 0) {
      blocks.push(
        <p key={key++} style={{ margin: '3px 0', lineHeight: 1.6, wordBreak: 'break-word' }}>
          {renderInlineMarkdown(paraLines.join('\n'))}
        </p>
      );
    }
  }

  return blocks;
}

// ── Conversation Message ──

const ConversationMessage: React.FC<{ turn: ConversationTurn; isLast?: boolean }> = ({ turn, isLast }) => {
  const isUser = turn.role === 'user';

  if (isUser) {
    return (
      <div style={{
        display: 'flex',
        justifyContent: 'flex-end',
        marginBottom: 12,
        animation: 'claudeFadeIn 0.2s ease-out',
      }}>
        <div style={{
          maxWidth: '80%',
          padding: '8px 14px',
          borderRadius: '16px 16px 4px 16px',
          backgroundColor: colors.userBubble,
          border: `1px solid ${colors.userBubbleBorder}`,
        }}>
          <pre style={{
            margin: 0,
            fontSize: '0.8rem',
            lineHeight: 1.6,
            color: colors.text,
            fontFamily: 'var(--claude-mono-font, monospace)',
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
          }}>
            {turn.content || '(empty)'}
          </pre>
        </div>
      </div>
    );
  }

  // Assistant message
  return (
    <div style={{
      marginBottom: 12,
      animation: 'claudeFadeIn 0.2s ease-out',
    }}>
      {/* Inline tool calls for this turn */}
      {turn.toolCalls && turn.toolCalls.length > 0 && (
        <InlineWorkLog toolCalls={turn.toolCalls} />
      )}
      <div style={{
        paddingLeft: 4,
        fontSize: '0.8rem',
        lineHeight: 1.6,
        color: colors.text,
      }}>
        {parseMarkdownBlocks(turn.content || '(empty)')}
      </div>
    </div>
  );
};

// ── Turn Divider ──

const TurnDivider: React.FC<{ label?: string }> = ({ label = 'Response' }) => (
  <div style={{
    display: 'flex',
    alignItems: 'center',
    gap: 10,
    margin: '16px 0 12px 0',
  }}>
    <div style={{ flex: 1, height: 1, backgroundColor: colors.divider }} />
    <span style={{ fontSize: '0.6rem', color: colors.mutedDim, fontWeight: 500, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
      {label}
    </span>
    <div style={{ flex: 1, height: 1, backgroundColor: colors.divider }} />
  </div>
);

// ── Approval Prompt ──

const ApprovalPrompt: React.FC<{ approval: PendingApproval; onRespond: (response: string) => void }> = ({ approval, onRespond }) => (
  <div style={{
    padding: '12px 14px',
    margin: '8px 0',
    borderRadius: 10,
    backgroundColor: 'rgba(248, 113, 113, 0.06)',
    border: `1px solid rgba(248, 113, 113, 0.2)`,
    animation: 'claudeFadeIn 0.2s ease-out',
  }}>
    <div style={{ fontSize: '0.75rem', color: colors.error, fontWeight: 600, marginBottom: 6 }}>
      Permission Required: {approval.toolName}
    </div>
    <pre style={{
      fontSize: '0.7rem',
      color: 'rgb(180, 180, 200)',
      margin: '4px 0 8px 0',
      padding: 8,
      backgroundColor: 'rgba(0, 0, 0, 0.3)',
      borderRadius: 6,
      maxHeight: 120,
      overflow: 'auto',
      whiteSpace: 'pre-wrap',
      fontFamily: 'var(--claude-mono-font, monospace)',
      border: `1px solid ${colors.border}`,
    }}>
      {JSON.stringify(approval.toolInput, null, 2)}
    </pre>
    <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
      <button style={{...approvalBtnStyle(colors.success), position: 'relative', zIndex: 10}} onClick={(e) => { e.stopPropagation(); console.log('[ApprovalBtn] Allow clicked'); onRespond('y'); }}>Allow</button>
      <button style={{...approvalBtnStyle(colors.error), position: 'relative', zIndex: 10}} onClick={(e) => { e.stopPropagation(); console.log('[ApprovalBtn] Deny clicked'); onRespond('n'); }}>Deny</button>
    </div>
  </div>
);

function approvalBtnStyle(color: string): React.CSSProperties {
  return {
    fontSize: '0.7rem',
    fontWeight: 600,
    padding: '4px 16px',
    borderRadius: 6,
    border: `1px solid ${color}`,
    backgroundColor: 'transparent',
    color,
    cursor: 'pointer',
  };
}

// ── Inline context sections (files, subagents) ──

const InlineFilesSection: React.FC<{ fileChanges: FileChange[] }> = ({ fileChanges }) => {
  const [expanded, setExpanded] = useState(false);
  if (fileChanges.length === 0) return null;

  return (
    <div style={{
      margin: '4px 0 8px 0',
      borderRadius: 8,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
    }}>
      <div
        onClick={() => setExpanded(!expanded)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '5px 10px',
          cursor: 'pointer',
          fontSize: '0.7rem',
          color: colors.muted,
          userSelect: 'none',
        }}
      >
        <span style={{
          display: 'inline-block',
          width: 10,
          fontSize: '0.55rem',
          transition: 'transform 0.15s',
          transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)',
        }}>
          {'\u25B6'}
        </span>
        <span>{fileChanges.length} file{fileChanges.length !== 1 ? 's' : ''} changed</span>
      </div>
      {expanded && (
        <div style={{ padding: '0 10px 6px 10px' }}>
          {fileChanges.slice(-20).map((fc, i) => {
            const filename = fc.path.split('/').pop() ?? fc.path;
            return (
              <div key={i} style={{ fontSize: '0.7rem', padding: '1px 0', display: 'flex', gap: 6, alignItems: 'center' }}>
                <span style={{ color: fc.toolName === 'Write' ? colors.success : colors.warning, fontWeight: 600, width: 10, textAlign: 'center' }}>
                  {fc.toolName === 'Write' ? '+' : '~'}
                </span>
                <span style={{ color: colors.text, fontFamily: 'monospace' }}>{filename}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

const InlineSubagentsSection: React.FC<{ subagents: SubagentInfo[] }> = ({ subagents }) => {
  const [expanded, setExpanded] = useState(false);
  if (subagents.length === 0) return null;

  return (
    <div style={{
      margin: '4px 0 8px 0',
      borderRadius: 8,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
    }}>
      <div
        onClick={() => setExpanded(!expanded)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '5px 10px',
          cursor: 'pointer',
          fontSize: '0.7rem',
          color: colors.muted,
          userSelect: 'none',
        }}
      >
        <span style={{
          display: 'inline-block',
          width: 10,
          fontSize: '0.55rem',
          transition: 'transform 0.15s',
          transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)',
        }}>
          {'\u25B6'}
        </span>
        <span>{subagents.length} subagent{subagents.length !== 1 ? 's' : ''}</span>
      </div>
      {expanded && (
        <div style={{ padding: '0 10px 6px 10px' }}>
          {subagents.map(sub => (
            <div key={sub.id} style={{ fontSize: '0.7rem', color: colors.text, display: 'flex', gap: 6, alignItems: 'center', padding: '1px 0' }}>
              <span>{sub.status === 'running' ? '\u23F3' : '\u2713'}</span>
              <span style={{ fontWeight: 600, color: '#c084fc' }}>{sub.type}</span>
              <span style={{ color: colors.mutedDim }}>{sub.id.slice(0, 8)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

// ── Scroll to Bottom Button ──

const ScrollToBottomButton: React.FC<{ onClick: () => void }> = ({ onClick }) => (
  <div style={{
    position: 'absolute',
    bottom: 12,
    left: '50%',
    transform: 'translateX(-50%)',
    animation: 'claudeScrollBtn 0.2s ease-out',
    zIndex: 10,
  }}>
    <button
      onClick={onClick}
      style={{
        fontSize: '0.65rem',
        fontWeight: 500,
        padding: '4px 14px',
        borderRadius: 20,
        border: `1px solid ${colors.border}`,
        backgroundColor: 'rgba(13, 13, 16, 0.9)',
        color: colors.muted,
        cursor: 'pointer',
        backdropFilter: 'blur(8px)',
        display: 'flex',
        alignItems: 'center',
        gap: 4,
      }}
    >
      <span style={{ fontSize: '0.7rem' }}>{'\u2193'}</span>
      Scroll to bottom
    </button>
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
  const [viewMode, setViewMode] = useState<ViewMode>('terminal');
  const [inputValue, setInputValue] = useState('');
  const [showScrollBtn, setShowScrollBtn] = useState(false);
  const [approvalDismissedAt, setApprovalDismissedAt] = useState(0);
  const [cancelledAt, setCancelledAt] = useState(0);
  const termContainerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const termInitRef = useRef(false);
  const conversationEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  const { config } = useConfig();
  const termCfg = config.terminal;

  // Inject keyframes
  useEffect(() => { ensureKeyframes(); }, []);

  // Set CSS variable for mono font
  useEffect(() => {
    if (typeof document !== 'undefined') {
      document.documentElement.style.setProperty('--claude-mono-font', termCfg.fontFamily || 'monospace');
    }
  }, [termCfg.fontFamily]);

  const handleExit = useCallback(() => {
    if (terminalRef.current) {
      terminalRef.current.write('\r\n\x1b[90m[Claude session exited]\x1b[0m\r\n');
    }
  }, []);

  const { sessionId, isReady, write, resize, attachToTerminal } = usePTY({
    paneId,
    shell: '__claude__',
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

  // Initialize xterm.js
  useEffect(() => {
    const container = termContainerRef.current;
    if (!container || termInitRef.current) return;
    termInitRef.current = true;

    // Ensure Nerd Fonts are loaded before canvas rendering
    const primaryFont = termCfg.fontFamily.split(',')[0].trim().replace(/"/g, '');
    document.fonts.load(`${termCfg.fontSize}px "${primaryFont}"`).catch(() => {});

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

    // Use GPU-accelerated WebGL renderer for better font/glyph support
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => { webgl.dispose(); });
      term.loadAddon(webgl);
    } catch (e) {
      console.warn('[ClaudePane] WebGL init failed, using canvas fallback:', e);
    }

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
  }, [viewMode, isActive, isReady]);

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
    const container = scrollContainerRef.current;
    if (!container) return;
    // Only auto-scroll if user is near the bottom
    const isNearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 100;
    if (isNearBottom) {
      conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [session?.conversation?.length, session?.activeToolCalls?.length]);

  // Track scroll position for "scroll to bottom" button
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const distFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    setShowScrollBtn(distFromBottom > 150);
  }, []);

  const scrollToBottom = useCallback(() => {
    conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, []);

  // Send approval response to Claude Code's interactive select menu.
  // The menu highlights "Yes" by default — Enter selects it.
  // For deny, arrow down to "No" (3rd item) then Enter.
  const handleApprovalRespond = useCallback((response: string) => {
    console.log(`[ClaudePane] sending approval: "${response}"`);
    if (response === 'y') {
      // "Yes" is already highlighted — just press Enter
      write('\r');
    } else {
      // Navigate down to "No" (skip "Yes, and don't ask again") then Enter
      write('\x1b[B\x1b[B\r');
    }
    setApprovalDismissedAt(Date.now());
  }, [write]);

  // Handle send
  const handleSend = useCallback(() => {
    if (inputValue.trim()) {
      write(inputValue + '\r');
      setInputValue('');
    }
  }, [inputValue, write]);

  // ── Derived data ──

  const activeToolCalls = session?.activeToolCalls ?? [];
  const completedToolCalls = session?.completedToolCalls ?? [];
  const conversation = session?.conversation ?? [];
  const fileChanges = session?.fileChanges ?? [];
  const subagents = session?.subagents ?? [];
  const pendingApproval = session?.pendingApproval ?? null;
  const serverStreaming = session?.ambientState === 'thinking' || session?.ambientState === 'streaming';
  // If user cancelled, suppress streaming UI until a new activity cycle begins
  const isStreaming = serverStreaming && (session?.lastActivity ?? 0) > cancelledAt;

  // Cancel the current task — send Escape and suppress streaming UI
  const cancelTask = useCallback(() => {
    write('\x1b');
    setCancelledAt(Date.now());
  }, [write]);

  // Escape key cancels in GUI mode (must be after cancelTask/isStreaming declarations)
  useEffect(() => {
    if (viewMode !== 'gui' || !isActive) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isStreaming) {
        e.preventDefault();
        cancelTask();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [viewMode, isActive, isStreaming, cancelTask]);

  // Only show in-progress tool calls globally — completed ones are per-turn
  const liveToolCalls = useMemo(() => {
    return [...completedToolCalls, ...activeToolCalls];
  }, [completedToolCalls, activeToolCalls]);

  // Build rendered conversation with dividers
  const renderedConversation = useMemo(() => {
    const items: React.ReactNode[] = [];
    let prevRole: string | null = null;

    conversation.forEach((turn, i) => {
      // Add divider when switching from user to assistant
      if (turn.role === 'assistant' && prevRole === 'user' && i > 0) {
        items.push(<TurnDivider key={`div-${i}`} />);
      }
      items.push(
        <ConversationMessage
          key={`msg-${i}`}
          turn={turn}
          isLast={i === conversation.length - 1}
        />
      );
      prevRole = turn.role;
    });

    return items;
  }, [conversation]);

  return (
    <div style={{
      width: '100%',
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      backgroundColor: colors.bg,
      color: colors.text,
      fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
    }}>
      {/* Toolbar */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '3px 10px',
        backgroundColor: colors.bgToolbar,
        borderBottom: `1px solid ${colors.border}`,
        minHeight: 26,
        flexShrink: 0,
      }}>
        <StatusBadge session={session} />
        <WorkingTimer session={session} />

        {cwd && (
          <span style={{
            fontSize: '0.55rem',
            color: colors.mutedDim,
            fontFamily: 'monospace',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            maxWidth: 200,
          }} title={cwd}>
            {cwd.split('/').pop() || cwd}
          </span>
        )}

        {session && (
          <span style={{ fontSize: '0.55rem', color: colors.mutedDim }}>
            {session.totalToolCalls} tools
          </span>
        )}

        {subagents.filter(s => s.status === 'running').length > 0 && (
          <span style={{ fontSize: '0.55rem', color: '#c084fc' }}>
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
              backgroundColor: viewMode === 'gui' ? 'rgba(96, 165, 250, 0.15)' : 'transparent',
              color: viewMode === 'gui' ? colors.accent : colors.mutedDim,
            }}
          >
            GUI
          </button>
          <button
            onClick={() => setViewMode('terminal')}
            style={{
              ...toggleBtnStyle,
              backgroundColor: viewMode === 'terminal' ? 'rgba(96, 165, 250, 0.15)' : 'transparent',
              color: viewMode === 'terminal' ? colors.accent : colors.mutedDim,
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
            flexDirection: 'column',
            overflow: 'hidden',
          }}>
            {/* Conversation scroll area */}
            <div
              ref={scrollContainerRef}
              onScroll={handleScroll}
              style={{
                flex: 1,
                overflowY: 'auto',
                padding: '12px 16px',
                position: 'relative',
              }}
            >
              {/* Centered content container */}
              <div style={{
                maxWidth: 720,
                margin: '0 auto',
              }}>
                {/* Empty states */}
                {conversation.length === 0 && !session && (
                  <div style={{ textAlign: 'center', marginTop: 60, color: colors.mutedDim }}>
                    <div style={{ fontSize: '2rem', marginBottom: 12, opacity: 0.4 }}>{'\u25C6'}</div>
                    <div style={{ fontSize: '0.8rem', color: colors.muted }}>Claude Code session starting...</div>
                    <div style={{ fontSize: '0.7rem', marginTop: 6, color: colors.mutedDim }}>
                      Waiting for hook events. Make sure hooks are configured in ~/.claude/settings.json
                    </div>
                  </div>
                )}

                {conversation.length === 0 && session && (
                  <div style={{ textAlign: 'center', marginTop: 60, color: colors.mutedDim }}>
                    <div style={{ fontSize: '2rem', marginBottom: 12, opacity: 0.4 }}>{'\u25C6'}</div>
                    <div style={{ fontSize: '0.8rem', color: colors.muted }}>Session connected</div>
                    <div style={{ fontSize: '0.7rem', marginTop: 6, color: colors.mutedDim }}>
                      Waiting for conversation activity...
                    </div>
                  </div>
                )}

                {/* Rendered conversation messages with dividers */}
                {renderedConversation}

                {/* Active tool calls as inline work log */}
                {liveToolCalls.length > 0 && (
                  <InlineWorkLog toolCalls={liveToolCalls} />
                )}

                {/* Inline file changes */}
                <InlineFilesSection fileChanges={fileChanges} />

                {/* Inline subagents */}
                <InlineSubagentsSection subagents={subagents} />

                {/* Pending approval — hide after user responds, show again for new approvals */}
                {pendingApproval && pendingApproval.timestamp > approvalDismissedAt && (
                  <ApprovalPrompt approval={pendingApproval} onRespond={handleApprovalRespond} />
                )}

                {/* Streaming indicator with cancel */}
                {isStreaming && (
                  <div style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '8px 0 4px 0',
                  }}>
                    <StreamingDots />
                    <button
                      onClick={cancelTask}
                      style={{
                        fontSize: '0.65rem',
                        fontWeight: 500,
                        padding: '2px 10px',
                        border: `1px solid ${colors.muted}`,
                        borderRadius: 4,
                        backgroundColor: 'transparent',
                        color: colors.muted,
                        cursor: 'pointer',
                        fontFamily: 'inherit',
                      }}
                      title="Cancel (Esc)"
                    >
                      Cancel
                    </button>
                  </div>
                )}

                <div ref={conversationEndRef} />
              </div>
            </div>

            {/* Scroll to bottom button */}
            {showScrollBtn && <ScrollToBottomButton onClick={scrollToBottom} />}

            {/* Composer / Input area */}
            <div style={{
              borderTop: `1px solid ${colors.border}`,
              padding: '8px 16px 10px 16px',
              flexShrink: 0,
            }}>
              <div style={{
                maxWidth: 720,
                margin: '0 auto',
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '6px 6px 6px 16px',
                borderRadius: 20,
                border: `1px solid ${colors.border}`,
                backgroundColor: 'rgba(255, 255, 255, 0.03)',
              }}>
                <input
                  placeholder="Message Claude..."
                  value={inputValue}
                  onChange={(e) => setInputValue(e.target.value)}
                  style={{
                    flex: 1,
                    fontSize: '0.8rem',
                    padding: '4px 0',
                    border: 'none',
                    backgroundColor: 'transparent',
                    color: colors.text,
                    outline: 'none',
                    fontFamily: 'inherit',
                    lineHeight: 1.4,
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && !e.shiftKey) {
                      e.preventDefault();
                      handleSend();
                    }
                  }}
                />
                <button
                  onClick={handleSend}
                  style={{
                    width: 28,
                    height: 28,
                    borderRadius: '50%',
                    border: 'none',
                    backgroundColor: inputValue.trim() ? colors.accent : 'rgba(255,255,255,0.06)',
                    color: inputValue.trim() ? '#0d0d10' : colors.mutedDim,
                    cursor: inputValue.trim() ? 'pointer' : 'default',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    fontSize: '0.85rem',
                    fontWeight: 700,
                    flexShrink: 0,
                    transition: 'background-color 0.15s, color 0.15s',
                  }}
                  aria-label="Send message"
                >
                  {'\u2191'}
                </button>
              </div>
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
  borderRadius: 4,
  border: 'none',
  cursor: 'pointer',
};

export default ClaudePane;
