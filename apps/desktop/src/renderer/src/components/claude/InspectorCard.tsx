import React, { useEffect, useMemo, useState } from 'react';
import { CheckCircle2, Circle, Loader2 } from 'lucide-react';
import type { ClaudeSessionSnapshot, FileChange, PlanStep } from '../../types/claudeSession';
import { claudeColors as colors, ensureKeyframes } from '../claude-shared';
import { WorkflowRunCard } from './WorkflowRunCard';
import { SubagentRow } from './SubagentRow';
import { fmtTokens } from './agentUtils';
import { planProgress } from '../../lib/sessionStats';
import { requestReviewFile } from '../../lib/reviewBus';
import { requestAgentWatch } from '../../lib/watchBus';

export type RailTab = 'files' | 'plan' | 'workflows' | 'agents' | 'usage';

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
      byPath.set(fc.path, {
        path: fc.path,
        name,
        dir: segs.slice(-2).join('/'),
        count: 1,
        lastTool: fc.toolName,
      });
    }
  }
  return Array.from(byPath.values()).reverse(); // most recent first
}

const fmtReset = (epochSec?: number): string => {
  if (!epochSec) return '';
  const d = new Date(epochSec * 1000);
  return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
};

// `pct` is optional: many accounts report a window's reset time without a
// utilization %. In that case we show the label + `sub` (the reset) and an empty
// meter track rather than hiding the window entirely.
const UsageBar: React.FC<{ label: string; pct?: number; sub?: string }> = ({ label, pct, sub }) => {
  const color =
    pct === undefined
      ? colors.muted
      : pct >= 80
        ? colors.error
        : pct >= 50
          ? colors.warning
          : colors.success;
  return (
    <div style={{ marginBottom: 10 }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontSize: '0.72rem',
          color: colors.muted,
          marginBottom: 3,
        }}
      >
        <span>{label}</span>
        <span style={{ color, fontVariantNumeric: 'tabular-nums' }}>
          {pct !== undefined ? `${Math.round(pct)}%` : sub ? '' : 'ok'}
          {pct !== undefined && sub ? ` · ${sub}` : pct === undefined && sub ? sub : ''}
        </span>
      </div>
      <div
        style={{
          height: 4,
          borderRadius: 2,
          backgroundColor: 'rgba(255,255,255,0.06)',
          overflow: 'hidden',
        }}
      >
        {pct !== undefined && (
          <div
            style={{
              width: `${Math.min(100, Math.max(0, pct))}%`,
              height: '100%',
              backgroundColor: color,
              transition: 'width 0.3s',
            }}
          />
        )}
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

/** Per-step glyph: check = completed, spinner = in_progress, hollow = pending. */
const StepGlyph: React.FC<{ status: PlanStep['status'] }> = ({ status }) => {
  if (status === 'completed')
    return <CheckCircle2 size={15} strokeWidth={2.2} style={{ color: colors.success }} />;
  if (status === 'in_progress')
    return (
      <Loader2
        size={15}
        strokeWidth={2.2}
        style={{ color: colors.accent, animation: 'claudeSpinner 1s linear infinite' }}
      />
    );
  return <Circle size={15} strokeWidth={2} style={{ color: colors.mutedDim }} />;
};

/** The Plan tab body: one row per step with a status glyph. The in_progress
 *  step surfaces its activeForm ("doing now") line; completed steps are muted. */
const PlanChecklist: React.FC<{ steps: PlanStep[] }> = ({ steps }) => {
  useEffect(() => {
    ensureKeyframes();
  }, []);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
      {steps.map((step, i) => {
        const active = step.status === 'in_progress';
        const done = step.status === 'completed';
        return (
          <div
            key={i}
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              gap: 8,
              padding: '5px 4px',
              margin: '0 -4px',
              borderRadius: 4,
              backgroundColor: active ? 'var(--wks-accent-bg)' : 'transparent',
            }}
          >
            <span style={{ flexShrink: 0, marginTop: 1, lineHeight: 0 }}>
              <StepGlyph status={step.status} />
            </span>
            <div style={{ minWidth: 0, flex: 1 }}>
              <div
                style={{
                  fontSize: '0.8rem',
                  lineHeight: 1.4,
                  color: done ? colors.mutedDim : active ? colors.textBright : colors.text,
                  textDecoration: done ? 'line-through' : 'none',
                  fontWeight: active ? 600 : 400,
                }}
              >
                {step.content}
              </div>
              {active && step.activeForm && (
                <div
                  style={{
                    fontSize: '0.72rem',
                    lineHeight: 1.4,
                    color: colors.accent,
                    marginTop: 2,
                  }}
                >
                  {step.activeForm}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
};

/**
 * The Inspector's full content — Plan, Flows (workflow runs), Agents (subagents),
 * Files and Usage — as a self-contained, props-driven card. Everything renders
 * purely from the passed `snapshot`, so the same card powers the docked
 * {@link InspectorRail}, a standalone `inspector` pane, a Fleet-Deck card
 * expansion and the sidebar hover peek. Live updates arrive by the `snapshot`
 * prop changing; the card never fetches on its own.
 *
 * The snapshot carries every field the card reads (plan, workflows, subagents,
 * fileChanges, usage, statusLine, cwd, totalToolCalls), so no section needs data
 * beyond it — an absent field just renders that section's empty state.
 */
export const InspectorCard: React.FC<{
  snapshot: ClaudeSessionSnapshot | null | undefined;
  /** Optional agent label shown as a slim header above the tab strip (docked
   *  rail passes none, so its chrome is unchanged). */
  agentName?: string;
  /** Denser paddings + smaller strip — for the hover peek popover. */
  compact?: boolean;
  /** Force the initially-selected tab. When omitted the card opens on whatever
   *  is actively happening (running flow → running agent → active plan → files). */
  initialTab?: RailTab;
  /** Rendered at the trailing edge of the tab strip (e.g. the rail's close ×). */
  headerAccessory?: React.ReactNode;
  /** Extra styles for the outer flex column (height caps for the peek, etc.). */
  style?: React.CSSProperties;
}> = ({ snapshot, agentName, compact, initialTab, headerAccessory, style }) => {
  const session = snapshot ?? null;
  const subagents = session?.subagents ?? [];
  const workflows = session?.workflows ?? [];
  const fileChanges = session?.fileChanges ?? [];
  const plan = session?.plan;
  const liveWorkflows = workflows.filter((w) => w.status === 'running').length;
  const liveSubagents = subagents.filter((s) => s.status === 'running').length;
  const planStats = planProgress(plan);

  // Open on whatever's actively happening: a running workflow wins, then a
  // running subagent. A plan that's actively in progress opens next — but only
  // when no diffs are competing for attention, so we don't steal from Files on
  // a session that's already changing code. Else the files list. An explicit
  // initialTab overrides this heuristic.
  const [tab, setTab] = useState<RailTab>(
    initialTab ??
      (liveWorkflows > 0
        ? 'workflows'
        : liveSubagents > 0
          ? 'agents'
          : planStats?.active && fileChanges.length === 0
            ? 'plan'
            : 'files'),
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
  const ctxPct =
    sl?.contextUsedPct ??
    (usage && usage.contextLimit > 0
      ? (usage.contextTokens / usage.contextLimit) * 100
      : undefined);
  const inTok = sl?.totalInputTokens ?? usage?.totalInputTokens;
  const outTok = sl?.totalOutputTokens ?? usage?.totalOutputTokens;
  const cost = sl?.costUSD ?? usage?.costUSD;
  const model = sl?.modelDisplay ?? usage?.model ?? undefined;

  const tabs: { id: RailTab; label: string; badge?: number | string }[] = [
    { id: 'files', label: 'Files', badge: files.length || undefined },
    {
      id: 'plan',
      label: 'Plan',
      badge: planStats ? `${planStats.done}/${planStats.total}` : undefined,
    },
    { id: 'workflows', label: 'Flows', badge: workflows.length || undefined },
    { id: 'agents', label: 'Agents', badge: subagents.length || undefined },
    { id: 'usage', label: 'Usage' },
  ];

  const bodyPad = compact ? '6px 8px' : '8px 10px';

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        height: '100%',
        overflow: 'hidden',
        ...style,
      }}
    >
      {agentName && (
        <div
          style={{
            padding: compact ? '5px 8px 3px' : '7px 10px 4px',
            fontSize: '0.72rem',
            fontWeight: 700,
            color: colors.textBright,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            flexShrink: 0,
            borderBottom: `1px solid ${colors.borderSubtle}`,
          }}
          title={agentName}
        >
          {agentName}
        </div>
      )}

      {/* Tab strip */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 2,
          padding: compact ? '3px 5px' : '4px 6px',
          borderBottom: `1px solid ${colors.borderSubtle}`,
          flexShrink: 0,
        }}
      >
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            style={{
              fontSize: compact ? '0.68rem' : '0.72rem',
              fontWeight: 600,
              padding: compact ? '3px 6px' : '4px 8px',
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
              <span
                style={{
                  fontSize: '0.64rem',
                  padding: '0 5px',
                  borderRadius: 6,
                  backgroundColor: 'rgba(255,255,255,0.08)',
                  color: t.id === 'agents' || t.id === 'workflows' ? colors.purple : colors.muted,
                  fontVariantNumeric: 'tabular-nums',
                }}
              >
                {t.badge}
              </span>
            )}
          </button>
        ))}
        <div style={{ flex: 1 }} />
        {headerAccessory}
      </div>

      {/* Tab body */}
      <div style={{ flex: 1, overflowY: 'auto', padding: bodyPad }}>
        {tab === 'files' &&
          (files.length === 0 ? (
            <div style={emptyStateStyle}>No files changed yet</div>
          ) : (
            files.map((f) => (
              <div
                key={f.path}
                title={`${f.path}\n\nClick to open in Review`}
                onClick={() =>
                  requestReviewFile({ path: f.path, cwd: session?.liveCwd ?? session?.cwd })
                }
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
                onMouseEnter={(e) => {
                  e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.backgroundColor = 'transparent';
                }}
              >
                <span
                  style={{
                    color: f.lastTool === 'Write' ? colors.success : colors.warning,
                    fontWeight: 700,
                    width: 10,
                    textAlign: 'center',
                    flexShrink: 0,
                  }}
                >
                  {f.lastTool === 'Write' ? '+' : '~'}
                </span>
                <span
                  style={{
                    color: colors.text,
                    fontFamily: 'var(--claude-mono-font, monospace)',
                    fontSize: '0.76rem',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    minWidth: 0,
                  }}
                >
                  {f.name}
                </span>
                <span
                  style={{
                    color: colors.mutedDim,
                    fontSize: '0.68rem',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {f.dir}
                </span>
                <div style={{ flex: 1 }} />
                {f.count > 1 && (
                  <span style={{ color: colors.mutedDim, fontSize: '0.68rem', flexShrink: 0 }}>
                    ×{f.count}
                  </span>
                )}
              </div>
            ))
          ))}

        {tab === 'plan' &&
          (!planStats ? (
            <div style={emptyStateStyle}>No plan yet</div>
          ) : (
            <>
              <div
                style={{
                  fontSize: '0.68rem',
                  color: planStats.done === planStats.total ? colors.success : colors.accent,
                  fontWeight: 600,
                  textTransform: 'uppercase',
                  letterSpacing: '0.05em',
                  marginBottom: 8,
                }}
              >
                {planStats.done}/{planStats.total} done
              </div>
              <PlanChecklist steps={plan!.steps} />
            </>
          ))}

        {tab === 'workflows' &&
          (workflows.length === 0 ? (
            <div style={emptyStateStyle}>No workflows running</div>
          ) : (
            <>
              {liveWorkflows > 0 && (
                <div
                  style={{
                    fontSize: '0.68rem',
                    color: 'var(--wks-purple)',
                    fontWeight: 600,
                    textTransform: 'uppercase',
                    letterSpacing: '0.05em',
                    marginBottom: 6,
                  }}
                >
                  {liveWorkflows} running
                </div>
              )}
              {workflows.map((run) => (
                <WorkflowRunCard
                  key={run.runId}
                  run={run}
                  onWatch={() => watchWorkflow(run.runId, run.name)}
                />
              ))}
            </>
          ))}

        {tab === 'agents' &&
          (subagents.length === 0 ? (
            <div style={emptyStateStyle}>No subagents yet</div>
          ) : (
            <>
              <div style={{ display: 'flex', alignItems: 'center', marginBottom: 6 }}>
                {liveSubagents > 0 && (
                  <span
                    style={{
                      fontSize: '0.68rem',
                      color: 'var(--wks-purple)',
                      fontWeight: 600,
                      textTransform: 'uppercase',
                      letterSpacing: '0.05em',
                    }}
                  >
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
                  onMouseEnter={(e) => {
                    e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)';
                    e.currentTarget.style.color = colors.text;
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.backgroundColor = 'transparent';
                    e.currentTarget.style.color = colors.muted;
                  }}
                >
                  Fleet view ↗
                </button>
              </div>
              {subagents.map((sub) => (
                <SubagentRow key={sub.id} sub={sub} onOpen={() => watchSubagent(sub)} />
              ))}
            </>
          ))}

        {tab === 'usage' &&
          (!sl && !usage ? (
            <div style={emptyStateStyle}>No usage data yet</div>
          ) : (
            <div>
              {model && (
                <div
                  style={{
                    fontSize: '0.78rem',
                    color: colors.textBright,
                    fontWeight: 600,
                    marginBottom: 10,
                  }}
                >
                  {model}
                </div>
              )}
              {sl?.capabilities &&
                (() => {
                  const c = sl.capabilities;
                  const chips: string[] = [];
                  if (c.fastMode) chips.push('⚡ Fast');
                  if (c.apiKeySource && c.apiKeySource !== 'none')
                    chips.push(`key: ${c.apiKeySource}`);
                  if (c.outputStyle && c.outputStyle !== 'default')
                    chips.push(`style: ${c.outputStyle}`);
                  if (c.mcpServers) chips.push(`${c.mcpServers} MCP`);
                  if (c.skills) chips.push(`${c.skills} skills`);
                  if (c.plugins) chips.push(`${c.plugins} plugins`);
                  if (c.agents) chips.push(`${c.agents} agents`);
                  if (c.memoryFiles) chips.push(`${c.memoryFiles} memory`);
                  if (!chips.length) return null;
                  return (
                    <div
                      style={{
                        display: 'flex',
                        flexWrap: 'wrap',
                        gap: 4,
                        marginBottom: 10,
                      }}
                    >
                      {chips.map((chip) => (
                        <span
                          key={chip}
                          style={{
                            fontSize: '0.66rem',
                            color: colors.muted,
                            background: 'rgba(255,255,255,0.05)',
                            borderRadius: 4,
                            padding: '1px 6px',
                            whiteSpace: 'nowrap',
                          }}
                        >
                          {chip}
                        </span>
                      ))}
                    </div>
                  );
                })()}
              {ctxPct !== undefined && <UsageBar label="Context window" pct={ctxPct} />}
              {(sl?.fiveHourPct !== undefined || sl?.fiveHourResetsAt !== undefined) && (
                <UsageBar
                  label="5-hour limit"
                  pct={sl?.fiveHourPct}
                  sub={sl?.fiveHourResetsAt ? `resets ${fmtReset(sl.fiveHourResetsAt)}` : undefined}
                />
              )}
              {(sl?.sevenDayPct !== undefined || sl?.sevenDayResetsAt !== undefined) && (
                <UsageBar
                  label="7-day limit"
                  pct={sl?.sevenDayPct}
                  sub={sl?.sevenDayResetsAt ? `resets ${fmtReset(sl.sevenDayResetsAt)}` : undefined}
                />
              )}
              {(sl?.monthlyPct !== undefined || sl?.monthlyResetsAt !== undefined) && (
                <UsageBar
                  label="Monthly limit"
                  pct={sl?.monthlyPct}
                  sub={sl?.monthlyResetsAt ? `resets ${fmtReset(sl.monthlyResetsAt)}` : undefined}
                />
              )}
              {sl?.rateLimitWarning && (
                <div style={{ fontSize: '0.72rem', color: colors.warning, marginBottom: 6 }}>
                  ⚠ {sl.rateLimitWarning}
                </div>
              )}
              {sl?.overageOutOfCredits && (
                <div style={{ fontSize: '0.72rem', color: colors.muted, marginBottom: 6 }}>
                  Monthly overage: out of credits
                </div>
              )}
              <div
                style={{
                  fontSize: '0.76rem',
                  color: colors.muted,
                  lineHeight: 1.6,
                  marginTop: 4,
                  fontVariantNumeric: 'tabular-nums',
                }}
              >
                {inTok !== undefined && (
                  <div>
                    Input tokens
                    <span style={{ float: 'right', color: colors.text }}>
                      {fmtTokens(inTok) || '0'}
                    </span>
                  </div>
                )}
                {outTok !== undefined && (
                  <div>
                    Output tokens
                    <span style={{ float: 'right', color: colors.text }}>
                      {fmtTokens(outTok) || '0'}
                    </span>
                  </div>
                )}
                {cost !== undefined && Number.isFinite(cost) && (
                  <div>
                    Cost
                    <span style={{ float: 'right', color: colors.text }}>${cost.toFixed(2)}</span>
                  </div>
                )}
                {session && (
                  <div>
                    Tool calls
                    <span style={{ float: 'right', color: colors.text }}>
                      {session.totalToolCalls}
                    </span>
                  </div>
                )}
                {session && (session.compactionCount ?? 0) > 0 && (
                  <div title="Context compactions this session (frequent = context churn)">
                    Compactions
                    <span style={{ float: 'right', color: colors.text }}>
                      {session.compacting ? `${session.compactionCount}· now` : session.compactionCount}
                    </span>
                  </div>
                )}
              </div>
            </div>
          ))}
      </div>
    </div>
  );
};
