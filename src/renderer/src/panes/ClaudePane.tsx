import React, { useEffect, useRef, useCallback, useState, useMemo } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebFontsAddon } from '@xterm/addon-web-fonts';
import '@xterm/xterm/css/xterm.css';
import { useClaudeSpawn } from '../hooks/useClaudeSpawn';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { useConfig } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import type { ClaudeSessionSnapshot, ToolCall, ConversationTurn, FileChange, PendingApproval, PendingQuestion, SubagentInfo, WorkflowRunInfo, WorkflowAgentInfo } from '../types/claudeSession';
import {
  claudeColors as colors,
  ensureKeyframes,
  StatusBadge,
  statusBadgeStyle as badgeStyle,
  formatToolSummary,
  WorkLogEntry,
  approvalBtnStyle,
  StreamingDots,
  sendApproval,
} from '../components/claude-shared';
import { parseMarkdownBlocks } from '../components/markdown';
import { RefreshCw } from '../components/icons';

/** Ensure each CSS font-family name with spaces is quoted */
function quoteFontFamily(ff: string): string {
  return ff.split(',').map(f => {
    f = f.trim();
    if (!f) return f;
    if (/^["']/.test(f) || /^(monospace|sans-serif|serif|cursive|fantasy|system-ui)$/i.test(f)) return f;
    if (f.includes(' ')) return `"${f}"`;
    return f;
  }).join(', ');
}

interface ClaudePaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  cwd?: string;
  profileId?: string;
  resumeSessionId?: string;
  /** If set, this pane is a viewer for an already-running daemon session. */
  attachSessionId?: string;
  /** Text to seed the message input with on first mount (library spawn). */
  initialPrompt?: string;
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
}

type ViewMode = 'gui' | 'terminal';

/** Number of conversation turns rendered per page (oldest load on scroll-up) */
const CONVERSATION_PAGE_SIZE = 60;

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

// ── Inline Work Log (Claude Code style — flat list, no card) ──

const AGENT_PURPLE = '#c084fc';

const fmtTokens = (n?: number): string => {
  if (!n) return '';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k`;
  return String(n);
};

const fmtDuration = (ms?: number): string => {
  if (ms === undefined || ms < 0) return '';
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, '0')}s`;
};

/** "claude-sonnet-4-6" to "sonnet-4-6" (enough to tell agents apart at a glance) */
const shortModel = (m?: string): string =>
  m ? m.replace(/^claude-/, '').replace(/-\d{8}$/, '') : '';

/** 1s ticker so elapsed clocks advance between session snapshots */
function useNowTicker(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const iv = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(iv);
  }, [active]);
  return active ? now : Date.now();
}

const AgentSpinner: React.FC<{ color?: string }> = ({ color = AGENT_PURPLE }) => (
  <span style={{
    display: 'inline-block', width: 12, height: 12,
    border: `1.5px solid ${color}`, borderTopColor: 'transparent',
    borderRadius: '50%', animation: 'claudeSpinner 0.8s linear infinite', flexShrink: 0,
  }} />
);

const agentStatusIcon = (status: WorkflowAgentInfo['status']): React.ReactNode => {
  switch (status) {
    case 'queued':
      return <span style={{ color: colors.mutedDim, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'\u25cc'}</span>;
    case 'running':
      return <AgentSpinner />;
    case 'failed':
      return <span style={{ color: colors.error, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'\u2717'}</span>;
    default:
      return <span style={{ color: colors.success, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'\u2713'}</span>;
  }
};

const agentMetaStyle: React.CSSProperties = {
  color: colors.mutedDim,
  fontSize: '0.65rem',
  fontVariantNumeric: 'tabular-nums',
  whiteSpace: 'nowrap',
  flexShrink: 0,
};

const lastToolLineStyle: React.CSSProperties = {
  paddingLeft: 18,
  fontSize: '0.68rem',
  color: colors.muted,
  lineHeight: 1.3,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
  fontFamily: 'var(--claude-mono-font, monospace)',
};

const WorkflowAgentRow: React.FC<{ agent: WorkflowAgentInfo; now: number }> = ({ agent, now }) => {
  const running = agent.status === 'running';
  const title = agent.label ?? agent.promptPreview ?? agent.id;
  const duration = agent.durationMs ?? (running && agent.startedAt ? now - agent.startedAt : undefined);

  return (
    <div style={{ padding: '1px 0' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: '0.72rem', lineHeight: 1.4 }}>
        {agentStatusIcon(agent.status)}
        <span
          title={agent.promptPreview ?? title}
          style={{
            color: running ? colors.textBright : colors.text,
            flex: 1,
            minWidth: 0,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {title}
        </span>
        {agent.model && <span style={agentMetaStyle}>{shortModel(agent.model)}</span>}
        {agent.tokens > 0 && <span style={agentMetaStyle}>{fmtTokens(agent.tokens)} tok</span>}
        {duration !== undefined && <span style={agentMetaStyle}>{fmtDuration(duration)}</span>}
      </div>
      {running && agent.lastToolName && (
        <div style={lastToolLineStyle}>
          {'\u2514'} {agent.lastToolName}{agent.lastToolSummary ? ` ${agent.lastToolSummary}` : ''}
        </div>
      )}
    </div>
  );
};

const WorkflowRunCard: React.FC<{ run: WorkflowRunInfo }> = ({ run }) => {
  const running = run.status === 'running';
  const [expanded, setExpanded] = useState(running);
  // Auto-collapse once when the run finishes; user can re-expand
  const prevStatus = useRef(run.status);
  useEffect(() => {
    if (prevStatus.current === 'running' && run.status !== 'running') setExpanded(false);
    prevStatus.current = run.status;
  }, [run.status]);

  const now = useNowTicker(running);
  const finished = run.agents.filter(a => a.status === 'done' || a.status === 'failed').length;
  const elapsed = running ? now - run.startedAt : run.durationMs;
  const tokens = run.totalTokens ?? run.agents.reduce((sum, a) => sum + a.tokens, 0);

  // Group agents by phase. phaseTitle is only known once the final state file
  // lands, so live agents render as a flat list until then.
  const groups = useMemo(() => {
    const out: { title: string | null; agents: WorkflowAgentInfo[] }[] = [];
    for (const a of run.agents) {
      const title = a.phaseTitle ?? null;
      const last = out[out.length - 1];
      if (last && last.title === title) last.agents.push(a);
      else out.push({ title, agents: [a] });
    }
    return out;
  }, [run.agents]);

  return (
    <div style={{
      margin: '4px 0',
      border: `1px solid ${colors.borderSubtle}`,
      borderRadius: 6,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
    }}>
      {/* Header */}
      <div
        onClick={() => setExpanded(e => !e)}
        style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '4px 8px', cursor: 'pointer', userSelect: 'none',
          fontSize: '0.72rem',
        }}
      >
        {running ? <AgentSpinner /> : (
          <span style={{ color: run.status === 'failed' ? colors.error : colors.success, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>
            {run.status === 'failed' ? '\u2717' : '\u2713'}
          </span>
        )}
        <span style={{ color: AGENT_PURPLE, fontWeight: 600, flexShrink: 0 }}>Workflow</span>
        <span
          title={run.description ?? run.name}
          style={{ color: colors.textBright, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}
        >
          {run.name ?? run.runId}
        </span>
        <div style={{ flex: 1 }} />
        <span style={agentMetaStyle}>{finished}/{run.agents.length} agents</span>
        {tokens > 0 && <span style={agentMetaStyle}>{fmtTokens(tokens)} tok</span>}
        {elapsed !== undefined && <span style={agentMetaStyle}>{fmtDuration(elapsed)}</span>}
        <span style={{ color: colors.mutedDim, fontSize: '0.6rem', flexShrink: 0 }}>{expanded ? '\u25be' : '\u25b8'}</span>
      </div>

      {/* Agent rows, grouped by phase when known */}
      {expanded && (
        <div style={{ padding: '2px 8px 6px 8px', borderTop: `1px solid ${colors.borderSubtle}` }}>
          {run.agents.length === 0 && (
            <div style={{ color: colors.mutedDim, fontSize: '0.68rem', padding: '2px 0' }}>
              starting agents...
            </div>
          )}
          {groups.map((g, gi) => (
            <div key={`${g.title ?? 'live'}-${gi}`}>
              {g.title && (
                <div style={{ color: colors.muted, fontSize: '0.62rem', fontWeight: 600, textTransform: 'uppercase', letterSpacing: 0.5, padding: '3px 0 1px 0' }}>
                  {g.title}
                </div>
              )}
              {g.agents.map(a => <WorkflowAgentRow key={a.id} agent={a} now={now} />)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

const SubagentRow: React.FC<{ sub: SubagentInfo }> = ({ sub }) => (
  <div style={{ padding: '1px 0' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: '0.75rem', lineHeight: 1.4 }}>
      {sub.status === 'running' ? <AgentSpinner /> : (
        <span style={{ color: colors.success, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'\u2713'}</span>
      )}
      <span style={{ color: AGENT_PURPLE, fontWeight: 600 }}>Agent</span>
      <span style={{ color: colors.text, flexShrink: 0 }}>{sub.type}</span>
      {sub.description ? (
        <span style={{ color: colors.muted, fontSize: '0.7rem', flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={sub.description}>
          {sub.description}
        </span>
      ) : (
        <div style={{ flex: 1 }} />
      )}
      {(sub.toolCalls ?? 0) > 0 && <span style={agentMetaStyle}>{sub.toolCalls} tools</span>}
      {(sub.tokens ?? 0) > 0 && <span style={agentMetaStyle}>{fmtTokens(sub.tokens)} tok</span>}
    </div>
    {sub.status === 'running' && sub.lastToolName && (
      <div style={lastToolLineStyle}>
        {'\u2514'} {sub.lastToolName}{sub.lastToolSummary ? ` ${sub.lastToolSummary}` : ''}
      </div>
    )}
  </div>
);

const InlineWorkLog: React.FC<{
  toolCalls: ToolCall[];
  subagents?: SubagentInfo[];
  workflows?: WorkflowRunInfo[];
}> = ({ toolCalls, subagents, workflows }) => {
  if (toolCalls.length === 0 && (!subagents || subagents.length === 0) && (!workflows || workflows.length === 0)) return null;

  return (
    <div style={{ margin: '4px 0 6px 0', padding: '0 2px' }}>
      {workflows && workflows.map(run => <WorkflowRunCard key={run.runId} run={run} />)}
      {subagents && subagents.map(sub => <SubagentRow key={sub.id} sub={sub} />)}
      {toolCalls.map(tc => <WorkLogEntry key={tc.id} tc={tc} />)}
    </div>
  );
};

/** Check if a tool call has diff-able content */
function hasDiff(tc: ToolCall): boolean {
  return (tc.name === 'Edit' || tc.name === 'MultiEdit') && (tc.input?.old_string || tc.input?.new_string);
}

/** Render a unified diff view with both old and new lines */
const DiffView: React.FC<{ oldStr: string; newStr: string; filePath?: string }> = ({ oldStr, newStr, filePath }) => {
  const fileName = filePath?.split(/[/\\]/).pop() ?? '';
  const oldLines = oldStr ? oldStr.split('\n') : [];
  const newLines = newStr ? newStr.split('\n') : [];
  const addedCount = newLines.length;
  const removedCount = oldLines.length;

  return (
    <div style={{
      margin: '6px 0',
      borderRadius: 6,
      overflow: 'hidden',
      border: `1px solid ${colors.borderSubtle}`,
      fontSize: '0.75rem',
      fontFamily: 'var(--claude-mono-font, monospace)',
    }}>
      {fileName && (
        <div style={{
          padding: '5px 12px',
          backgroundColor: 'rgba(255,255,255,0.03)',
          color: colors.text,
          fontSize: '0.72rem',
          fontWeight: 600,
          borderBottom: `1px solid ${colors.borderSubtle}`,
          display: 'flex',
          alignItems: 'center',
          gap: 8,
        }}>
          <span>{fileName}</span>
          {removedCount > 0 && <span style={{ color: colors.error, fontSize: '0.65rem', fontWeight: 400 }}>-{removedCount}</span>}
          {addedCount > 0 && <span style={{ color: colors.success, fontSize: '0.65rem', fontWeight: 400 }}>+{addedCount}</span>}
        </div>
      )}
      <div style={{ maxHeight: 600, overflow: 'auto' }}>
        {oldLines.map((line, i) => (
          <div key={`old-${i}`} style={{
            display: 'flex',
            backgroundColor: 'rgba(248, 113, 113, 0.08)',
            color: 'rgb(248, 150, 150)',
            lineHeight: 1.5,
          }}>
            <span style={{ color: 'rgba(248,150,150,0.4)', userSelect: 'none', width: 36, minWidth: 36, textAlign: 'right', padding: '1px 6px 1px 0', fontSize: '0.65rem', borderRight: '1px solid rgba(248,113,113,0.15)' }}>{i + 1}</span>
            <span style={{ color: colors.error, userSelect: 'none', width: 16, minWidth: 16, textAlign: 'center', padding: '1px 0' }}>-</span>
            <span style={{ padding: '1px 8px 1px 0', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{line}</span>
          </div>
        ))}
        {newLines.map((line, i) => (
          <div key={`new-${i}`} style={{
            display: 'flex',
            backgroundColor: 'rgba(74, 222, 128, 0.08)',
            color: 'rgb(150, 230, 170)',
            lineHeight: 1.5,
          }}>
            <span style={{ color: 'rgba(150,230,170,0.4)', userSelect: 'none', width: 36, minWidth: 36, textAlign: 'right', padding: '1px 6px 1px 0', fontSize: '0.65rem', borderRight: '1px solid rgba(74,222,128,0.15)' }}>{i + 1}</span>
            <span style={{ color: colors.success, userSelect: 'none', width: 16, minWidth: 16, textAlign: 'center', padding: '1px 0' }}>+</span>
            <span style={{ padding: '1px 8px 1px 0', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{line}</span>
          </div>
        ))}
      </div>
    </div>
  );
};

// ── Markdown rendering: shared module (components/markdown.tsx) ──



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
  const diffCalls = (turn.toolCalls ?? []).filter(tc => hasDiff(tc));
  const writeCalls = (turn.toolCalls ?? []).filter(tc => tc.name === 'Write' && tc.input?.content);

  return (
    <div style={{
      marginBottom: 12,
      animation: 'claudeFadeIn 0.2s ease-out',
    }}>
      {/* Collapsible tool call summary */}
      {turn.toolCalls && turn.toolCalls.length > 0 && (
        <InlineWorkLog toolCalls={turn.toolCalls} />
      )}

      {/* Inline diffs for Edit/MultiEdit — shown directly in chat */}
      {diffCalls.map(tc => (
        <DiffView
          key={tc.id}
          oldStr={tc.input?.old_string ?? ''}
          newStr={tc.input?.new_string ?? ''}
          filePath={tc.input?.file_path}
        />
      ))}

      {/* Inline file content for Write — shown directly in chat */}
      {writeCalls.map(tc => (
        <div key={tc.id} style={{
          margin: '6px 0',
          borderRadius: 6,
          overflow: 'hidden',
          border: `1px solid ${colors.borderSubtle}`,
          maxHeight: 600,
          overflowY: 'auto',
        }}>
          <div style={{
            padding: '4px 10px',
            backgroundColor: 'rgba(255,255,255,0.03)',
            color: colors.muted,
            fontSize: '0.65rem',
            borderBottom: `1px solid ${colors.borderSubtle}`,
            display: 'flex',
            alignItems: 'center',
            gap: 6,
          }}>
            <span style={{ color: colors.success }}>+</span>
            {tc.input?.file_path?.split(/[/\\]/).pop() ?? 'new file'}
          </div>
          <div style={{ margin: 0, fontSize: '0.7rem', fontFamily: 'var(--claude-mono-font, monospace)' }}>
            {tc.input.content.slice(0, 2000).split('\n').map((line: string, i: number) => (
              <div key={i} style={{ display: 'flex', lineHeight: 1.5 }}>
                <span style={{ color: 'rgba(150,230,170,0.35)', userSelect: 'none', width: 36, minWidth: 36, textAlign: 'right', padding: '0 6px 0 0', fontSize: '0.6rem', borderRight: '1px solid rgba(74,222,128,0.1)' }}>{i + 1}</span>
                <span style={{ color: 'rgb(150, 230, 170)', padding: '0 8px', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{line}</span>
              </div>
            ))}
            {tc.input.content.length > 2000 && (
              <div style={{ padding: '2px 8px 2px 44px', color: colors.muted, fontSize: '0.65rem' }}>...</div>
            )}
          </div>
        </div>
      ))}

      {turn.content ? (
        <div style={{
          paddingLeft: 4,
          fontSize: '0.8rem',
          lineHeight: 1.6,
          color: colors.text,
        }}>
          {parseMarkdownBlocks(turn.content)}
        </div>
      ) : null}
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

const ApprovalPrompt: React.FC<{ approval: PendingApproval; onRespond: (response: 'yes' | 'no') => void }> = ({ approval, onRespond }) => (
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
      <button style={{...approvalBtnStyle(colors.success), position: 'relative', zIndex: 10}} onClick={(e) => { e.stopPropagation(); onRespond('yes'); }}>Allow</button>
      <button style={{...approvalBtnStyle(colors.error), position: 'relative', zIndex: 10}} onClick={(e) => { e.stopPropagation(); onRespond('no'); }}>Deny</button>
    </div>
  </div>
);


// ── AskUserQuestion picker ──

const QuestionPicker: React.FC<{
  questions: PendingQuestion[];
  onAnswer: (payload: { option?: number; text?: string; answers?: string[] }) => void;
}> = ({ questions, onAnswer }) => {
  const [customText, setCustomText] = useState('');
  const single = questions.length === 1 ? questions[0] : null;

  return (
    <div style={{
      padding: '12px 14px',
      margin: '8px 0',
      borderRadius: 10,
      backgroundColor: 'var(--wks-accent-bg)',
      border: `1px solid ${colors.accent}`,
      animation: 'claudeFadeIn 0.2s ease-out',
    }}>
      {questions.map((q, qi) => (
        <div key={qi} style={{ marginBottom: qi < questions.length - 1 ? 12 : 0 }}>
          {q.header && (
            <div style={{ fontSize: '0.6rem', color: colors.mutedDim, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 4 }}>
              {q.header}
            </div>
          )}
          <div style={{ fontSize: '0.82rem', color: colors.textBright, fontWeight: 600, marginBottom: 8 }}>
            {q.question}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {q.options.map((opt, oi) => (
              <button
                key={oi}
                onClick={() => onAnswer({ option: oi + 1 })}
                style={{
                  textAlign: 'left',
                  padding: '6px 10px',
                  borderRadius: 6,
                  border: `1px solid ${colors.border}`,
                  backgroundColor: 'rgba(255,255,255,0.03)',
                  color: colors.text,
                  cursor: 'pointer',
                  fontFamily: 'inherit',
                  fontSize: '0.75rem',
                }}
              >
                <span style={{ color: colors.accent, fontWeight: 700, marginRight: 8 }}>{oi + 1}.</span>
                <span style={{ fontWeight: 600 }}>{opt.label}</span>
                {opt.description && (
                  <span style={{ color: colors.muted, marginLeft: 8, fontSize: '0.7rem' }}>
                    — {opt.description}
                  </span>
                )}
              </button>
            ))}
          </div>
        </div>
      ))}

      {single && (
        <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
          <input
            placeholder="Or type a custom answer..."
            value={customText}
            onChange={(e) => setCustomText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && customText.trim()) {
                onAnswer({ text: customText.trim() });
                setCustomText('');
              }
            }}
            style={{
              flex: 1,
              fontSize: '0.75rem',
              padding: '4px 8px',
              borderRadius: 4,
              border: `1px solid ${colors.border}`,
              backgroundColor: 'rgba(255,255,255,0.03)',
              color: colors.text,
              outline: 'none',
              fontFamily: 'inherit',
            }}
          />
          <button
            onClick={() => {
              if (customText.trim()) {
                onAnswer({ text: customText.trim() });
                setCustomText('');
              }
            }}
            disabled={!customText.trim()}
            style={{
              fontSize: '0.7rem',
              padding: '4px 12px',
              borderRadius: 4,
              border: `1px solid ${colors.accent}`,
              backgroundColor: customText.trim() ? colors.accent : 'transparent',
              color: customText.trim() ? '#0d0d10' : colors.muted,
              cursor: customText.trim() ? 'pointer' : 'default',
              fontFamily: 'inherit',
            }}
          >
            Send
          </button>
        </div>
      )}
    </div>
  );
};

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

// InlineSubagentsSection removed — subagents are now shown inside InlineWorkLog

// ── File Attachment Helpers ──

const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico', 'tiff']);
const PDF_EXTS = new Set(['pdf']);

interface AttachedFile {
  path: string;
  label: string; // "Image" | "PDF" | "File"
  name: string;  // basename
}

function classifyFile(filePath: string): AttachedFile {
  const name = filePath.split(/[/\\]/).pop() ?? filePath;
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  const label = IMAGE_EXTS.has(ext) ? 'Image' : PDF_EXTS.has(ext) ? 'PDF' : 'File';
  return { path: filePath, label, name };
}

function buildPromptPrefix(files: AttachedFile[]): string {
  return files.map(f => `[${f.label}: ${f.path}]`).join(' ') + ' ';
}

/** Extract file paths from a drop or paste event */
function extractFilePaths(dataTransfer: DataTransfer): string[] {
  const paths: string[] = [];
  if (dataTransfer.files) {
    for (let i = 0; i < dataTransfer.files.length; i++) {
      const f = dataTransfer.files[i] as File & { path?: string };
      if (f.path) paths.push(f.path);
    }
  }
  return paths;
}

// ── File Chips (shown above input when files are attached) ──

const FileChips: React.FC<{ files: AttachedFile[]; onRemove: (idx: number) => void }> = ({ files, onRemove }) => {
  if (files.length === 0) return null;
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, padding: '0 0 4px 0' }}>
      {files.map((f, i) => (
        <span key={i} style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 4,
          fontSize: '0.65rem',
          padding: '2px 8px',
          borderRadius: 10,
          backgroundColor: 'rgba(255,255,255,0.06)',
          border: `1px solid ${colors.borderSubtle}`,
          color: colors.text,
          maxWidth: 220,
        }}>
          <span style={{ color: f.label === 'Image' ? '#c084fc' : f.label === 'PDF' ? colors.error : colors.accent, fontWeight: 600 }}>
            {f.label === 'Image' ? '\u{1F5BC}' : f.label === 'PDF' ? '\u{1F4C4}' : '\u{1F4CE}'}
          </span>
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={f.path}>
            {f.name}
          </span>
          <span
            onClick={() => onRemove(i)}
            style={{ cursor: 'pointer', color: colors.muted, fontWeight: 700, fontSize: '0.7rem', marginLeft: 2 }}
          >
            {'\u00D7'}
          </span>
        </span>
      ))}
    </div>
  );
};

// ── Drop Overlay ──

const DropOverlay: React.FC = () => (
  <div style={{
    position: 'absolute',
    inset: 0,
    zIndex: 50,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: 'rgba(0, 0, 0, 0.6)',
    backdropFilter: 'blur(4px)',
    border: `2px dashed ${colors.accent}`,
    borderRadius: 8,
    pointerEvents: 'none',
  }}>
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontSize: '1.5rem', marginBottom: 8, opacity: 0.7 }}>{'\u{1F4CE}'}</div>
      <div style={{ fontSize: '0.8rem', color: colors.accent, fontWeight: 600 }}>Drop files here</div>
      <div style={{ fontSize: '0.65rem', color: colors.muted, marginTop: 4 }}>
        Images, code, PDFs — any file Claude can read
      </div>
    </div>
  </div>
);

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

const ClaudePane: React.FC<ClaudePaneProps> = ({ paneId, title, isActive, cwd, profileId, resumeSessionId, attachSessionId, initialPrompt, onPtyReady }) => {
  const [viewMode, setViewMode] = useState<ViewMode>(initialPrompt ? 'gui' : 'terminal');
  const [inputValue, setInputValue] = useState(initialPrompt ?? '');
  const [showScrollBtn, setShowScrollBtn] = useState(false);
  const [approvalDismissedAt, setApprovalDismissedAt] = useState(0);
  const [cancelledAt, setCancelledAt] = useState(0);
  const [visibleCount, setVisibleCount] = useState(CONVERSATION_PAGE_SIZE);
  const [attachedFiles, setAttachedFiles] = useState<AttachedFile[]>([]);
  const [isDragOver, setIsDragOver] = useState(false);
  const dragCounterRef = useRef(0);
  const termContainerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const termInitRef = useRef(false);
  const conversationEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const contentAreaRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Escape hatch for the rare backdrop-filter compositing garble: nudge the
  // content area onto a fresh raster (toggle a composited property for one
  // frame). Clears stale pixels without resetting scroll position or the PTY.
  const forceRepaint = useCallback(() => {
    const el = contentAreaRef.current;
    if (!el) return;
    el.style.transform = 'translateZ(0)';
    el.style.opacity = '0.999';
    requestAnimationFrame(() => {
      if (!el) return;
      el.style.transform = '';
      el.style.opacity = '';
    });
  }, []);

  const { config } = useConfig();
  const { terminalTheme } = useTheme();
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

  const { sessionId, isReady, write, resize, attachToTerminal, startSession } = useClaudeSpawn({
    paneId,
    cwd,
    profileId,
    resumeSessionId,
    attachSessionId,
    onExit: handleExit,
    defer: true,
  });

  const { session } = useClaudeSession({ ptySessionId: sessionId, active: isActive });

  // Enable the approval gateway in claudemon as soon as we have a session id
  // so PreToolUse hooks get parked for our UI to resolve.
  useEffect(() => {
    if (!sessionId) return;
    window.electronAPI.claudeGate(sessionId, true).catch(err =>
      console.warn('[ClaudePane] failed to enable approval gate:', err)
    );
  }, [sessionId]);

  // Notify parent of PTY session ID
  useEffect(() => {
    if (sessionId && onPtyReady) {
      onPtyReady(paneId, sessionId);
    }
  }, [sessionId, paneId, onPtyReady]);

  // Library: receive a prompt/skill inserted from the library. Targeted by
  // sessionId/paneId, or delivered to the active pane when untargeted.
  useEffect(() => {
    const handler = (e: Event) => {
      const d = (e as CustomEvent).detail as { text?: string; sessionId?: string; paneId?: string } | undefined;
      if (!d?.text) return;
      const targeted = d.sessionId || d.paneId;
      const matches = targeted ? (d.sessionId === sessionId || d.paneId === paneId) : isActive;
      if (!matches) return;
      setViewMode('gui');
      setInputValue((prev) => (prev.trim() ? `${prev.replace(/\s+$/, '')}\n${d.text}` : d.text!));
      requestAnimationFrame(() => inputRef.current?.focus());
    };
    window.addEventListener('library:insert', handler as EventListener);
    return () => window.removeEventListener('library:insert', handler as EventListener);
  }, [sessionId, paneId, isActive]);

  // Initialize xterm.js
  useEffect(() => {
    const container = termContainerRef.current;
    if (!container || termInitRef.current) return;
    termInitRef.current = true;

    const term = new Terminal({
      cursorBlink: termCfg.cursorBlink,
      fontSize: termCfg.fontSize,
      fontFamily: quoteFontFamily(termCfg.fontFamily),
      theme: terminalTheme,
      allowProposedApi: true,
      scrollback: termCfg.scrollback,
      convertEol: false,
      cursorStyle: termCfg.cursorStyle as 'block' | 'underline' | 'bar',
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
      startSession(term.cols, term.rows);
    });

    term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      // Ctrl+Shift+C — copy from terminal
      if (e.ctrlKey && e.shiftKey && e.key === 'C') {
        e.preventDefault();
        const sel = term.getSelection();
        if (sel) navigator.clipboard.writeText(sel);
        return false;
      }
      // Ctrl+Shift+V — paste
      if (e.ctrlKey && e.shiftKey && e.key === 'V') {
        e.preventDefault();
        navigator.clipboard.readText().then(text => { if (text) write(text); });
        return false;
      }
      // Ctrl+C — copy if selection, SIGINT if not
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'c') {
        const sel = term.getSelection();
        if (sel) { e.preventDefault(); navigator.clipboard.writeText(sel); term.clearSelection(); return false; }
        return true;
      }
      // Ctrl+V — paste
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'v') {
        e.preventDefault();
        navigator.clipboard.readText().then(text => { if (text) write(text); });
        return false;
      }
      if (e.ctrlKey && !e.altKey && !e.shiftKey && ['t', 'b', 'w', 'd', '/', '?', ',', 's', 'k'].includes(e.key)) return false;
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

  // Focus terminal or GUI input when pane becomes active
  useEffect(() => {
    if (!isActive) return;
    if (viewMode === 'terminal' && terminalRef.current) {
      terminalRef.current.focus();
      requestAnimationFrame(() => { try { fitAddonRef.current?.fit(); } catch {} });
    } else if (viewMode === 'gui') {
      requestAnimationFrame(() => inputRef.current?.focus());
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

  // Update terminal theme when it changes
  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.options.theme = terminalTheme;
    }
  }, [terminalTheme]);

  // Track scroll position for "scroll to bottom" button + lazy load older messages
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const distFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    setShowScrollBtn(distFromBottom > 150);
  }, []);

  const loadOlderMessages = useCallback(() => {
    const container = scrollContainerRef.current;
    const prevHeight = container?.scrollHeight ?? 0;
    setVisibleCount(prev => prev + CONVERSATION_PAGE_SIZE);
    // Preserve scroll position after DOM grows upward
    requestAnimationFrame(() => {
      if (container) {
        const newHeight = container.scrollHeight;
        container.scrollTop += newHeight - prevHeight;
      }
    });
  }, []);

  const scrollToBottom = useCallback(() => {
    conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, []);

  // ── File drag & drop ──

  // Global drag & drop — document + window level with dropEffect to tell
  // Electron/Chromium this is a valid drop target (prevents 🚫 cursor)
  useEffect(() => {
    const onDragOver = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    };
    const onDragEnter = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
      dragCounterRef.current++;
      if (e.dataTransfer?.types.includes('Files')) setIsDragOver(true);
    };
    const onDragLeave = (e: DragEvent) => {
      e.preventDefault();
      dragCounterRef.current--;
      if (dragCounterRef.current === 0) setIsDragOver(false);
    };
    const onDrop = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      dragCounterRef.current = 0;
      setIsDragOver(false);
      if (e.dataTransfer) {
        const paths = extractFilePaths(e.dataTransfer);
        console.log('[ClaudePane] drop paths:', paths);
        if (paths.length > 0) {
          setAttachedFiles(prev => [...prev, ...paths.map(classifyFile)]);
          setViewMode('gui');
        }
      }
    };

    // Register on both document and window for maximum coverage
    document.addEventListener('dragover', onDragOver, true);
    document.addEventListener('dragenter', onDragEnter, true);
    document.addEventListener('dragleave', onDragLeave, true);
    document.addEventListener('drop', onDrop, true);
    window.addEventListener('dragover', onDragOver);
    window.addEventListener('drop', onDrop);

    return () => {
      document.removeEventListener('dragover', onDragOver, true);
      document.removeEventListener('dragenter', onDragEnter, true);
      document.removeEventListener('dragleave', onDragLeave, true);
      document.removeEventListener('drop', onDrop, true);
      window.removeEventListener('dragover', onDragOver);
      window.removeEventListener('drop', onDrop);
    };
  }, []);

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const paths = extractFilePaths(e.clipboardData);
    if (paths.length > 0) {
      e.preventDefault();
      setAttachedFiles(prev => [...prev, ...paths.map(classifyFile)]);
    }
  }, []);

  const removeAttachedFile = useCallback((idx: number) => {
    setAttachedFiles(prev => prev.filter((_, i) => i !== idx));
  }, []);

  const openFilePicker = useCallback(async () => {
    const paths = await window.electronAPI.pickFiles(cwd);
    if (paths.length > 0) {
      setAttachedFiles(prev => [...prev, ...paths.map(classifyFile)]);
      if (viewMode === 'terminal') setViewMode('gui');
    }
  }, [cwd, viewMode]);

  const handleApprovalRespond = useCallback((response: 'yes' | 'no') => {
    if (!sessionId) return;
    // If a question picker is also pending (PermissionRequest racing with
    // AskUserQuestion's PreToolUse), the approval card is stale and shouldn't
    // do anything — the user actually wants to answer the picker. Writing a
    // keystroke fallback would select option 1 of the picker by accident.
    const hasPendingQuestion = (session?.pendingQuestions?.length ?? 0) > 0;
    window.electronAPI.claudeApprove(sessionId, response).catch(err => {
      console.warn('[ClaudePane] /approve failed:', err);
      if (!hasPendingQuestion) {
        sendApproval('', response === 'yes', write);
      } else {
        console.warn('[ClaudePane] suppressed keystroke fallback — question picker is active');
      }
    });
    setApprovalDismissedAt(Date.now());
  }, [sessionId, write, session?.pendingQuestions]);

  // Optimistic user messages (shown immediately before JSONL catches up).
  // We dequeue FIFO whenever session.conversation grows by a new user-message,
  // regardless of content — content-based matching was unreliable because
  // claude's JSONL records the post-input-processing text which can differ
  // from what we sent (whitespace, paste prefixes, autocomplete munging).
  const [optimisticMessages, setOptimisticMessages] = useState<ConversationTurn[]>([]);
  const [optimisticLoading, setOptimisticLoading] = useState(false);
  // Count of user-messages we've seen consumed by session.conversation.
  const consumedUserCountRef = useRef(0);

  // Handle send — detect issue keys, resolve context, then write to Claude's TUI
  const handleSend = useCallback(async () => {
    const hasFiles = attachedFiles.length > 0;
    const hasText = inputValue.trim().length > 0;
    if (!hasFiles && !hasText) return;

    const userText = inputValue.trim();
    setInputValue('');
    setAttachedFiles([]);

    // Build file prefix
    const filePrefix = hasFiles ? buildPromptPrefix(attachedFiles) : '';

    const fullMessage = filePrefix + userText;

    // Show message immediately and set loading state
    setOptimisticMessages(prev => [...prev, {
      role: 'user',
      content: fullMessage,
      timestamp: Date.now(),
    }]);
    setOptimisticLoading(true);

    // Prefer claudemon's /message endpoint (mode-gated, sends \r for us).
    // If the daemon reports the session isn't in input mode (e.g. OAuth picker
    // before SessionStart, picker in mid-flight) fall back to raw keystrokes.
    if (sessionId) {
      window.electronAPI.claudeMessage(sessionId, fullMessage).then((res) => {
        if (!res.ok) {
          console.warn(`[ClaudePane] /message rejected (mode=${res.mode}); falling back to PTY write`);
          write(fullMessage);
          setTimeout(() => write('\r'), 50);
        }
      }).catch(err => {
        console.warn('[ClaudePane] /message failed:', err);
        write(fullMessage);
        setTimeout(() => write('\r'), 50);
      });
    } else {
      write(fullMessage);
      setTimeout(() => write('\r'), 50);
    }
  }, [inputValue, write, attachedFiles, sessionId]);

  // Drop optimistic entries FIFO as session.conversation grows past the
  // count we last consumed. This avoids content-matching pitfalls.
  useEffect(() => {
    const userCount = (session?.conversation ?? []).filter(t => t.role === 'user').length;
    if (userCount > consumedUserCountRef.current) {
      const newlyConsumed = userCount - consumedUserCountRef.current;
      consumedUserCountRef.current = userCount;
      setOptimisticMessages(prev => (newlyConsumed >= prev.length ? [] : prev.slice(newlyConsumed)));
    }
    // Clear optimistic loading when server reports idle or we get a response
    if (optimisticLoading && (session?.ambientState === 'idle' || session?.ambientState === 'streaming')) {
      setOptimisticLoading(false);
    }
  }, [session?.conversation, session?.ambientState, optimisticLoading]);

  // ── Derived data ──

  const activeToolCalls = session?.activeToolCalls ?? [];
  const completedToolCalls = session?.completedToolCalls ?? [];
  const conversation = useMemo(() => {
    const base = session?.conversation ?? [];
    if (optimisticMessages.length === 0) return base;
    return [...base, ...optimisticMessages];
  }, [session?.conversation, optimisticMessages]);
  const hasOlderMessages = conversation.length > visibleCount;
  const fileChanges = session?.fileChanges ?? [];
  const subagents = session?.subagents ?? [];
  const workflows = session?.workflows ?? [];
  const pendingApproval = session?.pendingApproval ?? null;
  const pendingQuestions = session?.pendingQuestions ?? null;
  // Optimistic dismiss for the question picker — keeps the UI feeling snappy
  // even when /answer 409s and we fall back to a raw PTY write that takes
  // a moment to round-trip through the JSONL transcript.
  const [questionDismissedAt, setQuestionDismissedAt] = useState(0);

  const handleAnswer = useCallback((payload: { option?: number; text?: string; answers?: string[] }) => {
    if (!sessionId) return;
    setQuestionDismissedAt(Date.now());
    // We write directly to the PTY (via the MessagePort → /sessions/:id/input
    // path) instead of /sessions/:id/answer. /answer requires mode=Question,
    // which can race with concurrent hook events that flip the daemon's mode
    // back to Responding/Approval — and the renderer's view of "picker is up"
    // is what actually matters here. claude's own TUI picker accepts numeric
    // input + Enter the same way it accepts any other keystroke.
    if (payload.option !== undefined) {
      write(`${payload.option}\r`);
    } else if (payload.text !== undefined) {
      write(`${payload.text}\r`);
    } else if (payload.answers) {
      for (const ans of payload.answers) write(`${ans}\r`);
    }
  }, [sessionId, write]);
  const serverStreaming = optimisticLoading || session?.ambientState === 'thinking' || session?.ambientState === 'streaming';
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

  // Show active + completed tool calls, excluding any already in conversation
  // turns (from JSONL transcript) to avoid duplication while keeping history
  const liveToolCalls = useMemo(() => {
    const conversationToolIds = new Set<string>();
    for (const turn of conversation) {
      if (turn.toolCalls) {
        for (const tc of turn.toolCalls) {
          conversationToolIds.add(tc.id);
        }
      }
    }
    return [...activeToolCalls, ...completedToolCalls]
      .filter(tc => !conversationToolIds.has(tc.id));
  }, [activeToolCalls, completedToolCalls, conversation]);

  // Auto-scroll conversation to bottom (only when this pane is active —
  // scrollIntoView scrolls all ancestors, which would yank the outer
  // ScrollContainer back to this tab even when viewing another tab)
  useEffect(() => {
    if (!isActive) return;
    if (viewMode !== 'gui') return;
    const container = scrollContainerRef.current;
    if (!container) return;
    // Only auto-scroll if user is near the bottom
    const isNearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 100;
    if (isNearBottom) {
      conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [
    isActive,
    viewMode,
    session?.conversation?.length,
    session?.activeToolCalls?.length,
    session?.completedToolCalls?.length,
    session?.lastActivity,
    session?.subagents?.length,
    session?.workflows?.length,
    liveToolCalls.length,
    optimisticMessages.length,
    isStreaming,
  ]);

  // Build rendered conversation with dividers (windowed to last visibleCount turns)
  const renderedConversation = useMemo(() => {
    const items: React.ReactNode[] = [];
    const startIdx = Math.max(0, conversation.length - visibleCount);
    const visibleTurns = conversation.slice(startIdx);
    // Seed prevRole from turn before the window so the first divider renders correctly
    let prevRole: string | null = startIdx > 0 ? conversation[startIdx - 1].role : null;

    visibleTurns.forEach((turn, vi) => {
      const gi = startIdx + vi; // global index for stable keys
      if (turn.role === 'assistant' && prevRole === 'user' && gi > 0) {
        items.push(<TurnDivider key={`div-${gi}`} />);
      }
      items.push(
        <ConversationMessage
          key={`msg-${gi}`}
          turn={turn}
          isLast={gi === conversation.length - 1}
        />
      );
      prevRole = turn.role;
    });

    return items;
  }, [conversation, visibleCount]);

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
        <StatusBadge session={session} approvalDismissed={!!(pendingApproval && pendingApproval.timestamp <= approvalDismissedAt)} />
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

        {(() => {
          const liveAgents =
            subagents.filter(s => s.status === 'running').length +
            workflows.flatMap(w => w.agents).filter(a => a.status === 'running').length;
          return liveAgents > 0 ? (
            <span style={{ fontSize: '0.55rem', color: '#c084fc' }}>
              {liveAgents} subagent(s)
            </span>
          ) : null;
        })()}

        {attachedFiles.length > 0 && (
          <span style={{ fontSize: '0.55rem', color: colors.accent }}>
            {attachedFiles.length} file{attachedFiles.length !== 1 ? 's' : ''} attached
          </span>
        )}

        <div style={{ flex: 1 }} />

        {/* Redraw — clears the rare backdrop-filter compositing garble */}
        <button
          onClick={forceRepaint}
          title="Redraw pane (fixes occasional rendering glitches)"
          style={{
            ...toggleBtnStyle,
            display: 'flex',
            alignItems: 'center',
            backgroundColor: 'transparent',
            color: colors.mutedDim,
          }}
        >
          <RefreshCw size={13} strokeWidth={1.9} />
        </button>

        {/* Attach files */}
        <button
          onClick={openFilePicker}
          title="Attach files"
          style={{
            ...toggleBtnStyle,
            backgroundColor: 'transparent',
            color: colors.mutedDim,
            fontSize: '0.7rem',
          }}
        >
          +
        </button>

        {/* View mode toggle */}
        <div style={{ display: 'flex', gap: 2 }}>
          <button
            onClick={() => setViewMode('gui')}
            style={{
              ...toggleBtnStyle,
              backgroundColor: viewMode === 'gui' ? 'var(--wks-accent-bg)' : 'transparent',
              color: viewMode === 'gui' ? colors.accent : colors.mutedDim,
            }}
          >
            GUI
          </button>
          <button
            onClick={() => setViewMode('terminal')}
            style={{
              ...toggleBtnStyle,
              backgroundColor: viewMode === 'terminal' ? 'var(--wks-accent-bg)' : 'transparent',
              color: viewMode === 'terminal' ? colors.accent : colors.mutedDim,
            }}
          >
            Term
          </button>
        </div>
      </div>

      {/* Content area */}
      <div ref={contentAreaRef} style={{ flex: 1, overflow: 'hidden', position: 'relative' }}>
        {isDragOver && <DropOverlay />}

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
                // Promote to its own compositor layer so streaming/markdown
                // repaints don't corrupt the backdrop-filter snapshots of the
                // surrounding glass (transient garble that cleared on repaint).
                transform: 'translateZ(0)',
                contain: 'paint',
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

                {/* Load older messages */}
                {hasOlderMessages && (
                  <div style={{ textAlign: 'center', padding: '8px 0 12px 0' }}>
                    <button
                      onClick={loadOlderMessages}
                      style={{
                        fontSize: '0.65rem',
                        fontWeight: 500,
                        padding: '4px 16px',
                        borderRadius: 12,
                        border: `1px solid ${colors.border}`,
                        backgroundColor: 'rgba(255,255,255,0.03)',
                        color: colors.muted,
                        cursor: 'pointer',
                        fontFamily: 'inherit',
                      }}
                    >
                      Load {Math.min(CONVERSATION_PAGE_SIZE, conversation.length - visibleCount)} earlier messages
                      {' '}({conversation.length - visibleCount} hidden)
                    </button>
                  </div>
                )}

                {/* Rendered conversation messages with dividers */}
                {renderedConversation}

                {/* Active tool calls + subagents + workflow runs as inline work log */}
                {(liveToolCalls.length > 0 || subagents.length > 0 || workflows.length > 0) && (
                  <InlineWorkLog toolCalls={liveToolCalls} subagents={subagents} workflows={workflows} />
                )}

                {/* Inline file changes */}
                <InlineFilesSection fileChanges={fileChanges} />

                {/* Pending approval — hide after user responds, show again for new approvals.
                    A pending question picker always takes precedence: claude might fire
                    PermissionRequest in the same turn as an AskUserQuestion PreToolUse,
                    and the approval card from the former is stale once the picker is up. */}
                {pendingApproval && pendingApproval.timestamp > approvalDismissedAt && !(pendingQuestions && pendingQuestions.length > 0) && (
                  <ApprovalPrompt approval={pendingApproval} onRespond={handleApprovalRespond} />
                )}

                {/* AskUserQuestion picker — surfaced by claudemon's mode=question.
                    Hide after the user clicks an option until the next PreToolUse
                    arrives (which clears pendingQuestions server-side). */}
                {pendingQuestions && pendingQuestions.length > 0 && (session?.lastActivity ?? 0) > questionDismissedAt && (
                  <QuestionPicker questions={pendingQuestions} onAnswer={handleAnswer} />
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
              <div style={{ maxWidth: 720, margin: '0 auto' }}>
                <FileChips files={attachedFiles} onRemove={removeAttachedFile} />
                <div style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                  padding: '6px 6px 6px 10px',
                  borderRadius: 20,
                  border: `1px solid ${attachedFiles.length > 0 ? colors.accent : colors.border}`,
                  backgroundColor: 'rgba(255, 255, 255, 0.03)',
                  transition: 'border-color 0.15s',
                }}>
                  <button
                    onClick={openFilePicker}
                    title="Attach files"
                    style={{
                      width: 24,
                      height: 24,
                      borderRadius: '50%',
                      border: 'none',
                      backgroundColor: 'transparent',
                      color: colors.muted,
                      cursor: 'pointer',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      fontSize: '1rem',
                      flexShrink: 0,
                      padding: 0,
                    }}
                  >
                    +
                  </button>
                  <input
                    ref={inputRef}
                    placeholder={attachedFiles.length > 0 ? 'What should Claude do with these files?' : 'Message Claude...'}
                    value={inputValue}
                    onChange={(e) => setInputValue(e.target.value)}
                    onPaste={handlePaste}
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
                    backgroundColor: (inputValue.trim() || attachedFiles.length > 0) ? colors.accent : 'rgba(255,255,255,0.06)',
                    color: (inputValue.trim() || attachedFiles.length > 0) ? '#0d0d10' : colors.mutedDim,
                    cursor: (inputValue.trim() || attachedFiles.length > 0) ? 'pointer' : 'default',
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
