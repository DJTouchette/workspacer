import React, { useEffect, useMemo, useRef, useState } from 'react';
import type { ToolCall, SubagentInfo, WorkflowRunInfo } from '../../types/claudeSession';
import { claudeColors as colors, WorkLogEntry } from '../claude-shared';
import { DiffView, hasDiff, ReadView, hasRead } from './DiffView';
import { SubagentRow } from './SubagentRow';
import { WorkflowRunCard } from './WorkflowRunCard';
import { AgentSpinner } from './WorkflowAgentRow';
import { summarizeWork } from './WorkCard';
import { useNowTicker } from './useNowTicker';

/**
 * ToolTraceCard — the "Trace" work-log style: a run of tool calls rendered as
 * a waterfall monitor instead of a prose list. Each call is one row — a
 * category-colored tool chip, its target, a duration bar on the run's shared
 * time axis (parallel work and slow steps read at a glance), elapsed time and
 * status — and clicking a row expands the full input/response (diffs and read
 * excerpts included) for digging in.
 *
 * Drop-in alternative to WorkCard (same props); which one renders is the
 * `claude.workLog` setting ('cards' | 'trace').
 */

/** Category → chip/bar color. Literal fallbacks keep the trace legible if a
 *  theme doesn't define the var. */
const CATEGORY_COLOR: Record<string, string> = {
  read: 'var(--wks-accent-text, #60a5fa)',
  edit: 'var(--wks-success, #4ade80)',
  cmd: 'var(--wks-warning, #fbbf24)',
  search: 'var(--wks-purple, #c084fc)',
  agent: 'var(--wks-purple, #c084fc)',
  web: '#38bdf8',
  other: 'var(--wks-text-muted, #8c8c9b)',
};

function categoryOf(tc: ToolCall): keyof typeof CATEGORY_COLOR {
  const n = tc.name;
  if (n === 'Read' || n === 'NotebookRead') return 'read';
  if (n === 'Edit' || n === 'MultiEdit' || n === 'Write' || n === 'NotebookEdit') return 'edit';
  if (n === 'Bash' || n === 'PowerShell') return 'cmd';
  if (n === 'Grep' || n === 'Glob' || n === 'LS') return 'search';
  if (n === 'Agent' || n === 'Workflow' || n === 'Task') return 'agent';
  if (n === 'WebFetch' || n === 'WebSearch') return 'web';
  return 'other';
}

/** Short human target for the row: file basename, command, pattern, … */
function callTarget(tc: ToolCall): string {
  const i = tc.input ?? {};
  const base = (p: unknown) =>
    typeof p === 'string' ? (p.replace(/\\/g, '/').split('/').pop() ?? p) : '';
  switch (tc.name) {
    case 'Read': case 'Edit': case 'MultiEdit': case 'Write':
    case 'NotebookRead': case 'NotebookEdit':
      return base(i.file_path ?? i.notebook_path);
    case 'Bash': case 'PowerShell':
      return typeof i.command === 'string' ? i.command : '';
    case 'Grep': case 'Glob':
      return typeof i.pattern === 'string' ? i.pattern : '';
    case 'Agent': case 'Task':
      return typeof i.description === 'string' ? i.description : '';
    case 'Workflow':
      return typeof i.name === 'string' ? i.name : 'workflow';
    case 'WebFetch': case 'WebSearch':
      return typeof (i.url ?? i.query) === 'string' ? (i.url ?? i.query) : '';
    default: {
      const first = Object.values(i).find((v) => typeof v === 'string') as string | undefined;
      return first ?? '';
    }
  }
}

function fmtDuration(ms: number): string {
  if (ms < 0) return '';
  if (ms < 950) return `${(ms / 1000).toFixed(1)}s`;
  if (ms < 59_500) return `${Math.round(ms / 1000)}s`;
  const m = Math.floor(ms / 60_000);
  return `${m}m ${Math.round((ms % 60_000) / 1000)}s`;
}

/** Compact JSON for the dig-in panel — pretty, but bounded. */
function excerptJson(v: unknown, max = 4000): string {
  try {
    const s = typeof v === 'string' ? v : JSON.stringify(v, null, 2);
    return s.length > max ? `${s.slice(0, max)}\n… (${s.length - max} more chars)` : s;
  } catch {
    return String(v);
  }
}

const detailPre: React.CSSProperties = {
  margin: '4px 0 0 0',
  padding: '6px 8px',
  borderRadius: 6,
  background: 'rgba(0,0,0,0.25)',
  border: `1px solid ${colors.borderSubtle}`,
  fontFamily: 'var(--claude-mono-font, monospace)',
  fontSize: '0.66rem',
  lineHeight: 1.45,
  color: colors.text,
  whiteSpace: 'pre-wrap',
  wordBreak: 'break-word',
  maxHeight: 240,
  overflowY: 'auto',
};

const TraceRow: React.FC<{
  tc: ToolCall;
  t0: number;
  span: number;
  now: number;
  open: boolean;
  onToggle: () => void;
}> = ({ tc, t0, span, now, open, onToggle }) => {
  const cat = categoryOf(tc);
  const running = tc.status === 'running';
  const failed = tc.status === 'failed';
  const color = failed ? colors.error : CATEGORY_COLOR[cat];
  const end = tc.completedAt ?? (running ? now : tc.startedAt);
  const leftPct = Math.min(100, Math.max(0, ((tc.startedAt - t0) / span) * 100));
  const widthPct = Math.min(100 - leftPct, Math.max(0.75, ((end - tc.startedAt) / span) * 100));
  const dur = end - tc.startedAt;
  const target = callTarget(tc);

  return (
    <>
      <div
        onClick={onToggle}
        title={target || tc.name}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '2px 8px',
          cursor: 'pointer',
          userSelect: 'none',
          borderRadius: 5,
          background: open ? 'rgba(255,255,255,0.04)' : 'transparent',
        }}
        onMouseEnter={(e) => { if (!open) (e.currentTarget as HTMLDivElement).style.background = 'rgba(255,255,255,0.03)'; }}
        onMouseLeave={(e) => { if (!open) (e.currentTarget as HTMLDivElement).style.background = 'transparent'; }}
      >
        {/* Tool chip */}
        <span style={{
          flexShrink: 0,
          width: 64,
          fontSize: '0.62rem',
          fontWeight: 600,
          fontFamily: 'var(--claude-mono-font, monospace)',
          color,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}>
          {tc.name}
        </span>
        {/* Target */}
        <span style={{
          flexShrink: 1,
          minWidth: 60,
          maxWidth: '34%',
          fontSize: '0.66rem',
          fontFamily: 'var(--claude-mono-font, monospace)',
          color: colors.text,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}>
          {target}
        </span>
        {/* Waterfall lane */}
        <span style={{
          flex: 1,
          minWidth: 40,
          height: 8,
          position: 'relative',
          borderRadius: 4,
          background: 'rgba(255,255,255,0.045)',
          overflow: 'hidden',
        }}>
          <span style={{
            position: 'absolute',
            top: 0,
            bottom: 0,
            left: `${leftPct}%`,
            width: `${widthPct}%`,
            borderRadius: 4,
            background: color,
            opacity: running ? 0.9 : 0.65,
            animation: running ? 'wks-pulse 1.4s ease-in-out infinite' : undefined,
          }} />
        </span>
        {/* Duration */}
        <span style={{
          flexShrink: 0,
          width: 42,
          textAlign: 'right',
          fontSize: '0.62rem',
          fontVariantNumeric: 'tabular-nums',
          fontFamily: 'var(--claude-mono-font, monospace)',
          color: colors.muted,
        }}>
          {fmtDuration(dur)}
        </span>
        {/* Status */}
        <span style={{ flexShrink: 0, width: 14, textAlign: 'center', fontSize: '0.68rem' }}>
          {running
            ? <AgentSpinner />
            : <span style={{ color: failed ? colors.error : colors.success }}>{failed ? '✗' : '✓'}</span>}
        </span>
      </div>

      {open && (
        <div style={{
          margin: '0 8px 6px 8px',
          padding: '4px 8px 8px 8px',
          borderLeft: `2px solid ${color}`,
          background: 'rgba(255,255,255,0.02)',
          borderRadius: '0 6px 6px 0',
        }}>
          <WorkLogEntry tc={tc} />
          {hasDiff(tc) && (
            <DiffView
              oldStr={tc.input?.old_string ?? ''}
              newStr={tc.input?.new_string ?? ''}
              filePath={tc.input?.file_path}
            />
          )}
          {hasRead(tc) && (
            <ReadView response={String(tc.response)} filePath={tc.input?.file_path} />
          )}
          {!hasDiff(tc) && !hasRead(tc) && (
            <>
              {tc.input != null && Object.keys(tc.input ?? {}).length > 0 && (
                <pre style={detailPre}>{excerptJson(tc.input)}</pre>
              )}
              {tc.response != null && (
                <pre style={detailPre}>{excerptJson(tc.response)}</pre>
              )}
            </>
          )}
        </div>
      )}
    </>
  );
};

const ToolTraceCardInner: React.FC<{
  toolCalls: ToolCall[];
  subagentByToolId?: Map<string, SubagentInfo>;
  workflowByToolId?: Map<string, WorkflowRunInfo>;
  live?: boolean;
  isLast?: boolean;
  cwd?: string;
}> = ({ toolCalls, subagentByToolId, workflowByToolId, live, isLast }) => {
  const hasOrchestration = useMemo(
    () => toolCalls.some(tc => workflowByToolId?.has(tc.id) || subagentByToolId?.has(tc.id)),
    [toolCalls, workflowByToolId, subagentByToolId],
  );
  // Same open/collapse lifecycle as WorkCard: open while live/last, collapse
  // when superseded; orchestration cards stay closed (their rich run cards
  // surface below instead).
  const active = (!!live || !!isLast) && !hasOrchestration;
  const [expanded, setExpanded] = useState(active);
  const [openRows, setOpenRows] = useState<Set<string>>(() => new Set());
  const wasActive = useRef(active);
  useEffect(() => {
    if (wasActive.current && !active) { setExpanded(false); setOpenRows(new Set()); }
    else if (!wasActive.current && active) { setExpanded(true); }
    wasActive.current = active;
  }, [active]);

  const summary = useMemo(() => summarizeWork(toolCalls), [toolCalls]);
  const anyRunning = !!live || toolCalls.some(tc => tc.status === 'running');
  const now = useNowTicker(anyRunning && expanded);

  // Shared time axis across the run.
  const t0 = useMemo(
    () => toolCalls.reduce((m, tc) => Math.min(m, tc.startedAt), Number.MAX_SAFE_INTEGER),
    [toolCalls],
  );
  const t1 = toolCalls.reduce(
    (m, tc) => Math.max(m, tc.completedAt ?? (tc.status === 'running' ? now : tc.startedAt)),
    t0,
  );
  const span = Math.max(1, t1 - t0);

  const toggleRow = (id: string) =>
    setOpenRows(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });

  // Running orchestration cards surface even while collapsed (WorkCard parity).
  const visibleWhenCollapsed = useMemo(() => {
    if (expanded) return [];
    const out: React.ReactNode[] = [];
    for (const tc of toolCalls) {
      const wf = workflowByToolId?.get(tc.id);
      if (wf && wf.status === 'running') out.push(<WorkflowRunCard key={`wf-${tc.id}`} run={wf} />);
      const sub = subagentByToolId?.get(tc.id);
      if (sub && sub.status === 'running') out.push(<SubagentRow key={`sub-${tc.id}`} sub={sub} />);
    }
    return out;
  }, [expanded, toolCalls, subagentByToolId, workflowByToolId]);

  return (
    <div style={{
      margin: '4px 0 10px 0',
      borderRadius: 8,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.015)',
      overflow: 'hidden',
      animation: 'claudeFadeIn 0.2s ease-out',
    }}>
      <div
        onClick={() => setExpanded(e => !e)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '5px 10px',
          cursor: 'pointer',
          userSelect: 'none',
          fontSize: '0.7rem',
          color: colors.muted,
        }}
      >
        {anyRunning ? <AgentSpinner /> : (
          <span style={{
            color: summary.failed > 0 ? colors.error : colors.success,
            fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0,
          }}>
            {summary.failed > 0 ? '✗' : '✓'}
          </span>
        )}
        <span style={{ color: colors.text, fontWeight: 600, flexShrink: 0 }}>
          {toolCalls.length} step{toolCalls.length !== 1 ? 's' : ''}
        </span>
        <span style={{ flexShrink: 0, fontVariantNumeric: 'tabular-nums' }}>
          {fmtDuration(span)}
        </span>
        {summary.text && (
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>
            {summary.text}
          </span>
        )}
        <div style={{ flex: 1 }} />
        {(summary.added > 0 || summary.removed > 0) && (
          <span style={{ fontFamily: 'var(--claude-mono-font, monospace)', fontSize: '0.62rem', flexShrink: 0 }}>
            {summary.added > 0 && <span style={{ color: colors.success }}>+{summary.added}</span>}
            {summary.added > 0 && summary.removed > 0 && ' '}
            {summary.removed > 0 && <span style={{ color: colors.error }}>−{summary.removed}</span>}
          </span>
        )}
        <span style={{ color: colors.mutedDim, fontSize: '0.6rem', flexShrink: 0 }}>{expanded ? '▾' : '▸'}</span>
      </div>

      {visibleWhenCollapsed.length > 0 && (
        <div style={{ padding: '0 10px 6px 10px' }}>
          {visibleWhenCollapsed}
        </div>
      )}

      {expanded && (
        <div style={{ padding: '2px 2px 6px 2px', borderTop: `1px solid ${colors.borderSubtle}` }}>
          {toolCalls.map(tc => {
            const wf = workflowByToolId?.get(tc.id);
            if (wf) return <div key={tc.id} style={{ padding: '0 8px' }}><WorkflowRunCard run={wf} /></div>;
            const sub = subagentByToolId?.get(tc.id);
            if (sub) return <div key={tc.id} style={{ padding: '0 8px' }}><SubagentRow sub={sub} /></div>;
            return (
              <TraceRow
                key={tc.id}
                tc={tc}
                t0={t0}
                span={span}
                now={now}
                open={openRows.has(tc.id)}
                onToggle={() => toggleRow(tc.id)}
              />
            );
          })}
        </div>
      )}
    </div>
  );
};

export const ToolTraceCard = React.memo(ToolTraceCardInner);
