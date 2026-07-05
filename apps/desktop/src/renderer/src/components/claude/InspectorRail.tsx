import React, { useMemo, useState } from 'react';
import type { ClaudeSessionSnapshot, FileChange } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { WorkflowRunCard } from './WorkflowRunCard';
import { SubagentRow } from './SubagentRow';
import { fmtTokens } from './agentUtils';
import { requestReviewFile } from '../../lib/reviewBus';
import { requestAgentWatch } from '../../lib/watchBus';

type RailTab = 'files' | 'workflows' | 'agents' | 'usage';

/** One row per touched path: how many times and what touched it last. */
interface FileAgg {
  path: string;
  name: string;
  dir: string;
  count: number;
  lastTool: string;
}

function aggregateFiles(changes: FileChange[]): FileAgg[] {
  const byPath = new Map<string, FileAgg>();
  for (const fc of changes) {
    const segs = fc.path.split(/[/\\]/);
    const name = segs.pop() ?? fc.path;
    const existing = byPath.get(fc.path);
    if (existing) {
      existing.count++;
      existing.lastTool = fc.toolName;
      // Re-insert so the most recently touched file sorts last
      byPath.delete(fc.path);
      byPath.set(fc.path, existing);
    } else {
      byPath.set(fc.path, { path: fc.path, name, dir: segs.slice(-2).join('/'), count: 1, lastTool: fc.toolName });
    }
  }
  return Array.from(byPath.values()).reverse(); // most recent first
}

const fmtReset = (epochSec?: number): string => {
  if (!epochSec) return '';
  const d = new Date(epochSec * 1000);
  return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
};

const UsageBar: React.FC<{ label: string; pct: number; sub?: string }> = ({ label, pct, sub }) => {
  const color = pct >= 80 ? colors.error : pct >= 50 ? colors.warning : colors.success;
  return (
    <div style={{ marginBottom: 10 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.72rem', color: colors.muted, marginBottom: 3 }}>
        <span>{label}</span>
        <span style={{ color, fontVariantNumeric: 'tabular-nums' }}>{Math.round(pct)}%{sub ? ` · ${sub}` : ''}</span>
      </div>
      <div style={{ height: 4, borderRadius: 2, backgroundColor: 'rgba(255,255,255,0.06)', overflow: 'hidden' }}>
        <div style={{ width: `${Math.min(100, Math.max(0, pct))}%`, height: '100%', backgroundColor: color, transition: 'width 0.3s' }} />
      </div>
    </div>
  );
};

const emptyStateStyle: React.CSSProperties = {
  padding: '24px 12px',
  textAlign: 'center',
  color: colors.mutedDim,
  fontSize: '0.78rem',
};

/**
 * Right-hand inspector for the Claude pane GUI view: a persistent home for
 * the session's files, agents, and usage — so workflow/agent state doesn't
 * live only in the scrollback.
 */
export const InspectorRail: React.FC<{
  session: ClaudeSessionSnapshot | null;
  onClose: () => void;
}> = ({ session, onClose }) => {
  const subagents = session?.subagents ?? [];
  const workflows = session?.workflows ?? [];
  const fileChanges = session?.fileChanges ?? [];
  const liveWorkflows = workflows.filter(w => w.status === 'running').length;
  const liveSubagents = subagents.filter(s => s.status === 'running').length;

  // Open on whatever's actively happening: a running workflow wins, then a
  // running subagent, else the files list.
  const [tab, setTab] = useState<RailTab>(
    liveWorkflows > 0 ? 'workflows' : liveSubagents > 0 ? 'agents' : 'files',
  );
  const files = useMemo(() => aggregateFiles(fileChanges), [fileChanges]);

  // Click-through: open a dedicated live watch pane for one subagent /
  // workflow run (handled by App, which owns the tab manager).
  const sessionId = session?.sessionId;
  const watchSubagent = (sub: (typeof subagents)[number]) => {
    if (!sessionId) return;
    requestAgentWatch({
      sessionId,
      kind: 'subagent',
      id: sub.id,
      title: `Agent: ${sub.type}`,
    });
  };
  const watchWorkflow = (runId: string, name?: string) => {
    if (!sessionId) return;
    requestAgentWatch({ sessionId, kind: 'workflow', id: runId, title: `Flow: ${name ?? runId}` });
  };
  // Fleet view for plain (non-workflow) subagents: the same timeline surface a
  // workflow run gets, fed all of this session's Agent-tool subagents.
  const watchFleet = () => {
    if (!sessionId) return;
    requestAgentWatch({ sessionId, kind: 'agents', id: sessionId, title: 'Agents: fleet' });
  };

  const sl = session?.statusLine;
  const usage = session?.usage;
  const ctxPct = sl?.contextUsedPct
    ?? (usage && usage.contextLimit > 0 ? (usage.contextTokens / usage.contextLimit) * 100 : undefined);
  const inTok = sl?.totalInputTokens ?? usage?.totalInputTokens;
  const outTok = sl?.totalOutputTokens ?? usage?.totalOutputTokens;
  const cost = sl?.costUSD ?? usage?.costUSD;
  const model = sl?.modelDisplay ?? usage?.model ?? undefined;

  const tabs: { id: RailTab; label: string; badge?: number }[] = [
    { id: 'files', label: 'Files', badge: files.length || undefined },
    { id: 'workflows', label: 'Flows', badge: workflows.length || undefined },
    { id: 'agents', label: 'Agents', badge: subagents.length || undefined },
    { id: 'usage', label: 'Usage' },
  ];

  return (
    <div style={{
      width: 320,
      flexShrink: 0,
      display: 'flex',
      flexDirection: 'column',
      borderLeft: `1px solid ${colors.border}`,
      backgroundColor: 'rgba(255,255,255,0.012)',
      overflow: 'hidden',
    }}>
      {/* Tab strip */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 2,
        padding: '4px 6px',
        borderBottom: `1px solid ${colors.borderSubtle}`,
        flexShrink: 0,
      }}>
        {tabs.map(t => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            style={{
              fontSize: '0.72rem',
              fontWeight: 600,
              padding: '4px 8px',
              borderRadius: 4,
              border: 'none',
              cursor: 'pointer',
              backgroundColor: tab === t.id ? 'var(--wks-accent-bg)' : 'transparent',
              color: tab === t.id ? colors.accent : colors.muted,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 3,
              flexShrink: 0,
            }}
          >
            {t.label}
            {t.badge !== undefined && (
              <span style={{
                fontSize: '0.64rem',
                padding: '0 5px',
                borderRadius: 6,
                backgroundColor: 'rgba(255,255,255,0.08)',
                color: (t.id === 'agents' || t.id === 'workflows') ? '#c084fc' : colors.muted,
                fontVariantNumeric: 'tabular-nums',
              }}>
                {t.badge}
              </span>
            )}
          </button>
        ))}
        <div style={{ flex: 1 }} />
        <button
          onClick={onClose}
          title="Hide inspector"
          style={{
            fontSize: '0.8rem',
            border: 'none',
            background: 'transparent',
            color: colors.mutedDim,
            cursor: 'pointer',
            padding: '2px 6px',
          }}
        >
          ×
        </button>
      </div>

      {/* Tab body */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '8px 10px' }}>
        {tab === 'files' && (
          files.length === 0 ? (
            <div style={emptyStateStyle}>No files changed yet</div>
          ) : (
            files.map(f => (
              <div
                key={f.path}
                title={`${f.path}\n\nClick to open in Review`}
                onClick={() => requestReviewFile({ path: f.path, cwd: session?.cwd })}
                style={{
                  display: 'flex',
                  alignItems: 'baseline',
                  gap: 6,
                  padding: '4px 4px',
                  margin: '0 -4px',
                  borderRadius: 4,
                  fontSize: '0.8rem',
                  borderBottom: `1px solid ${colors.borderSubtle}`,
                  cursor: 'pointer',
                }}
                onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)'; }}
                onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = 'transparent'; }}
              >
                <span style={{
                  color: f.lastTool === 'Write' ? colors.success : colors.warning,
                  fontWeight: 700,
                  width: 10,
                  textAlign: 'center',
                  flexShrink: 0,
                }}>
                  {f.lastTool === 'Write' ? '+' : '~'}
                </span>
                <span style={{
                  color: colors.text,
                  fontFamily: 'var(--claude-mono-font, monospace)',
                  fontSize: '0.76rem',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  minWidth: 0,
                }}>
                  {f.name}
                </span>
                <span style={{ color: colors.mutedDim, fontSize: '0.68rem', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {f.dir}
                </span>
                <div style={{ flex: 1 }} />
                {f.count > 1 && (
                  <span style={{ color: colors.mutedDim, fontSize: '0.68rem', flexShrink: 0 }}>×{f.count}</span>
                )}
              </div>
            ))
          )
        )}

        {tab === 'workflows' && (
          workflows.length === 0 ? (
            <div style={emptyStateStyle}>No workflows running</div>
          ) : (
            <>
              {liveWorkflows > 0 && (
                <div style={{ fontSize: '0.68rem', color: '#c084fc', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 6 }}>
                  {liveWorkflows} running
                </div>
              )}
              {workflows.map(run => (
                <WorkflowRunCard key={run.runId} run={run} onWatch={() => watchWorkflow(run.runId, run.name)} />
              ))}
            </>
          )
        )}

        {tab === 'agents' && (
          subagents.length === 0 ? (
            <div style={emptyStateStyle}>No subagents yet</div>
          ) : (
            <>
              <div style={{ display: 'flex', alignItems: 'center', marginBottom: 6 }}>
                {liveSubagents > 0 && (
                  <span style={{ fontSize: '0.68rem', color: '#c084fc', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                    {liveSubagents} running
                  </span>
                )}
                <div style={{ flex: 1 }} />
                <button
                  onClick={watchFleet}
                  title="Open all this session’s agents on one timeline"
                  style={{
                    fontSize: '0.66rem',
                    fontWeight: 600,
                    padding: '2px 8px',
                    borderRadius: 4,
                    border: `1px solid ${colors.borderSubtle}`,
                    cursor: 'pointer',
                    backgroundColor: 'transparent',
                    color: colors.muted,
                    flexShrink: 0,
                  }}
                  onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)'; e.currentTarget.style.color = colors.text; }}
                  onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = 'transparent'; e.currentTarget.style.color = colors.muted; }}
                >
                  Fleet view ↗
                </button>
              </div>
              {subagents.map(sub => <SubagentRow key={sub.id} sub={sub} onOpen={() => watchSubagent(sub)} />)}
            </>
          )
        )}

        {tab === 'usage' && (
          (!sl && !usage) ? (
            <div style={emptyStateStyle}>No usage data yet</div>
          ) : (
            <div>
              {model && (
                <div style={{ fontSize: '0.78rem', color: colors.textBright, fontWeight: 600, marginBottom: 10 }}>
                  {model}
                </div>
              )}
              {ctxPct !== undefined && <UsageBar label="Context window" pct={ctxPct} />}
              {sl?.fiveHourPct !== undefined && (
                <UsageBar label="5-hour limit" pct={sl.fiveHourPct} sub={sl.fiveHourResetsAt ? `resets ${fmtReset(sl.fiveHourResetsAt)}` : undefined} />
              )}
              {sl?.sevenDayPct !== undefined && (
                <UsageBar label="7-day limit" pct={sl.sevenDayPct} sub={sl.sevenDayResetsAt ? `resets ${fmtReset(sl.sevenDayResetsAt)}` : undefined} />
              )}
              <div style={{ fontSize: '0.76rem', color: colors.muted, lineHeight: 1.9, marginTop: 4, fontVariantNumeric: 'tabular-nums' }}>
                {inTok !== undefined && <div>Input tokens<span style={{ float: 'right', color: colors.text }}>{fmtTokens(inTok) || '0'}</span></div>}
                {outTok !== undefined && <div>Output tokens<span style={{ float: 'right', color: colors.text }}>{fmtTokens(outTok) || '0'}</span></div>}
                {cost !== undefined && Number.isFinite(cost) && <div>Cost<span style={{ float: 'right', color: colors.text }}>${cost.toFixed(2)}</span></div>}
                {session && <div>Tool calls<span style={{ float: 'right', color: colors.text }}>{session.totalToolCalls}</span></div>}
              </div>
            </div>
          )
        )}
      </div>
    </div>
  );
};
