import React, { useContext, useEffect, useMemo, useState } from 'react';
import {
  CheckCircle2,
  Circle,
  Loader2,
  FileDiff,
  ListChecks,
  Workflow,
  Bot,
  Gauge,
  ArrowUpRight,
} from 'lucide-react';
import type { ClaudeSessionSnapshot, FileChange, PlanStep } from '../../types/claudeSession';
import {
  claudeColors as colors,
  ensureKeyframes,
  badgeColors,
  badgeLabels,
} from '../claude-shared';
import { WorkflowRunCard } from './WorkflowRunCard';
import { SubagentRow } from './SubagentRow';
import { fmtTokens } from './agentUtils';
import { planProgress } from '../../lib/sessionStats';
import { requestReviewFile } from '../../lib/reviewBus';
import { requestAgentWatch, requestContextPane } from '../../lib/watchBus';
import { ConfigContext } from '../../contexts/ConfigContext';

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

/** Per-session cost budget: a tiny inline dollar input tucked into the usage
 *  stats. Empty/0 disables it. When set, an OS notification fires (from the main
 *  process) once the session's spend crosses the threshold. Reads the config
 *  context directly (not the throwing hook) so contexts without a provider —
 *  tests, isolated embeds — simply omit the row. */
const BudgetRow: React.FC<{ sessionId: string; cost?: number }> = ({ sessionId, cost }) => {
  const ctx = useContext(ConfigContext);
  const budget = ctx?.config.claude?.budgets?.[sessionId];
  const [draft, setDraft] = useState(budget != null ? String(budget) : '');
  useEffect(() => {
    setDraft(budget != null ? String(budget) : '');
  }, [budget]);
  if (!ctx) return null;
  const { config, save } = ctx;

  const persist = (raw: string): void => {
    const n = parseFloat(raw);
    const budgets = { ...(config.claude?.budgets ?? {}) };
    if (!raw.trim() || !Number.isFinite(n) || n <= 0) delete budgets[sessionId];
    else budgets[sessionId] = n;
    void save({
      claude: { ...config.claude, defaultView: config.claude?.defaultView ?? 'terminal', budgets },
    });
  };

  const over = budget != null && cost !== undefined && cost >= budget;
  return (
    <div style={statRowStyle}>
      <span
        title="Notify once this session's spend crosses this amount"
        style={{ color: colors.muted }}
      >
        Budget
      </span>
      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
        {over && (
          <span
            style={{
              color: colors.error,
              fontSize: '0.62rem',
              fontWeight: 600,
              textTransform: 'uppercase',
              letterSpacing: '0.04em',
            }}
          >
            over
          </span>
        )}
        <span style={{ color: colors.mutedDim }}>$</span>
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={(e) => persist(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') persist((e.target as HTMLInputElement).value);
          }}
          placeholder="—"
          inputMode="decimal"
          style={{
            width: 46,
            textAlign: 'right',
            fontSize: '0.72rem',
            fontFamily: 'inherit',
            color: over ? colors.error : colors.text,
            background: 'rgba(255,255,255,0.05)',
            border: `1px solid ${colors.borderSubtle}`,
            borderRadius: 5,
            padding: '1px 5px',
            outline: 'none',
          }}
          onFocus={(e) => {
            e.currentTarget.style.borderColor = 'var(--wks-border-active, var(--wks-accent))';
          }}
          onBlurCapture={(e) => {
            e.currentTarget.style.borderColor = colors.borderSubtle;
          }}
        />
      </span>
    </div>
  );
};

// `pct` is optional: many accounts report a window's reset time without a
// utilization %. In that case we show the label + `sub` (the reset) and an
// empty meter track rather than hiding the window entirely.
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
    <div style={{ marginBottom: 11 }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          fontSize: '0.72rem',
          color: colors.muted,
          marginBottom: 4,
        }}
      >
        <span>{label}</span>
        {/* Value in text ink; the dot + fill carry the severity color. */}
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 5,
            color: pct !== undefined ? colors.textBright : colors.mutedDim,
            fontVariantNumeric: 'tabular-nums',
            fontWeight: pct !== undefined ? 600 : 400,
            fontSize: '0.7rem',
          }}
        >
          {pct !== undefined && (
            <span
              aria-hidden
              style={{ width: 6, height: 6, borderRadius: '50%', background: color }}
            />
          )}
          {pct !== undefined ? `${Math.round(pct)}%` : ''}
          {sub && (
            <span style={{ color: colors.mutedDim, fontWeight: 400 }}>
              {pct !== undefined ? `· ${sub}` : sub}
            </span>
          )}
        </span>
      </div>
      {/* Severity fill on a lighter step of the same hue. */}
      <div
        style={{
          height: 5,
          borderRadius: 2.5,
          background:
            pct !== undefined
              ? `color-mix(in srgb, ${color} 14%, transparent)`
              : 'rgba(255,255,255,0.06)',
          overflow: 'hidden',
        }}
      >
        {pct !== undefined && (
          <div
            style={{
              width: `${Math.min(100, Math.max(0, pct))}%`,
              height: '100%',
              borderRadius: '2.5px 2px 2px 2.5px',
              backgroundColor: color,
              transition: 'width 0.3s ease, background-color 0.3s ease',
            }}
          />
        )}
      </div>
    </div>
  );
};

/** Empty state: the tab's own icon, dimmed, above the (test-pinned) message. */
const EmptyState: React.FC<{
  icon: React.ComponentType<{ size?: number | string }>;
  text: string;
}> = ({ icon: Icon, text }) => (
  <div
    style={{
      padding: '28px 12px',
      textAlign: 'center',
      color: colors.mutedDim,
      fontSize: '0.76rem',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      gap: 8,
    }}
  >
    <span
      aria-hidden
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: 34,
        height: 34,
        borderRadius: '50%',
        background: 'rgba(255,255,255,0.04)',
        border: `1px solid ${colors.borderSubtle}`,
        color: colors.mutedDim,
      }}
    >
      <Icon size={16} />
    </span>
    {text}
  </div>
);

/** Small uppercase "N running" section marker with a live pulse dot. */
const RunningMarker: React.FC<{ count: number }> = ({ count }) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      fontSize: '0.66rem',
      color: colors.purple,
      fontWeight: 600,
      textTransform: 'uppercase',
      letterSpacing: '0.05em',
    }}
  >
    <span
      aria-hidden
      style={{
        width: 6,
        height: 6,
        borderRadius: '50%',
        background: colors.purple,
        animation: 'claudePulseDot 1.4s ease-in-out infinite',
      }}
    />
    {count} running
  </span>
);

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

/** The Plan tab body: one row per step with a status glyph, threaded on a
 *  hairline guide so the checklist reads as a route, not a list. The
 *  in_progress step surfaces its activeForm ("doing now") line. */
const PlanChecklist: React.FC<{ steps: PlanStep[] }> = ({ steps }) => {
  useEffect(() => {
    ensureKeyframes();
  }, []);
  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {steps.map((step, i) => {
        const active = step.status === 'in_progress';
        const done = step.status === 'completed';
        const last = i === steps.length - 1;
        return (
          <div
            key={i}
            style={{
              display: 'flex',
              alignItems: 'stretch',
              gap: 9,
              padding: '0 6px 0 4px',
              margin: '0 -4px',
              borderRadius: 6,
              backgroundColor: active ? 'var(--wks-accent-bg)' : 'transparent',
            }}
          >
            {/* Glyph column with the connecting guide line */}
            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                flexShrink: 0,
                width: 15,
              }}
            >
              <span style={{ lineHeight: 0, paddingTop: 6 }}>
                <StepGlyph status={step.status} />
              </span>
              {!last && (
                <span
                  aria-hidden
                  style={{
                    flex: 1,
                    width: 1,
                    background: done
                      ? 'color-mix(in srgb, var(--wks-success) 35%, transparent)'
                      : colors.borderSubtle,
                    marginTop: 2,
                    marginBottom: -4,
                    minHeight: 6,
                  }}
                />
              )}
            </div>
            <div style={{ minWidth: 0, flex: 1, padding: '5px 0 9px' }}>
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
 * expansion and the sidebar right-click Inspect. Live updates arrive by the
 * `snapshot` prop changing; the card never fetches on its own.
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
  /** Denser paddings + smaller strip — for embedded/expansion contexts. */
  compact?: boolean;
  /** Force the initially-selected tab. When omitted the card opens on whatever
   *  is actively happening (running flow → running agent → active plan → files). */
  initialTab?: RailTab;
  /** Rendered at the trailing edge of the tab strip (e.g. the rail's close ×). */
  headerAccessory?: React.ReactNode;
  /** Extra styles for the outer flex column (height caps for the peek, etc.). */
  style?: React.CSSProperties;
}> = ({ snapshot, agentName, compact, initialTab, headerAccessory, style }) => {
  useEffect(() => {
    ensureKeyframes();
  }, []);
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
  // Monitor view for plain (non-workflow) subagents: the same timeline surface a
  // workflow run gets, fed all of this session's Agent-tool subagents.
  const watchAgents = () => {
    if (!sessionId) return;
    requestAgentWatch({ sessionId, kind: 'agents', id: sessionId, title: 'Agent monitor' });
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

  const tabs: {
    id: RailTab;
    label: string;
    icon: React.ComponentType<{ size?: number | string; strokeWidth?: number | string }>;
    badge?: number | string;
    live?: boolean;
  }[] = [
    { id: 'files', label: 'Files', icon: FileDiff, badge: files.length || undefined },
    {
      id: 'plan',
      label: 'Plan',
      icon: ListChecks,
      badge: planStats ? `${planStats.done}/${planStats.total}` : undefined,
    },
    {
      id: 'workflows',
      label: 'Flows',
      icon: Workflow,
      badge: workflows.length || undefined,
      live: liveWorkflows > 0,
    },
    {
      id: 'agents',
      label: 'Agents',
      icon: Bot,
      badge: subagents.length || undefined,
      live: liveSubagents > 0,
    },
    { id: 'usage', label: 'Usage', icon: Gauge },
  ];

  const bodyPad = compact ? '8px 10px' : '10px 12px';
  const ambient = session?.ambientState;

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
            display: 'flex',
            alignItems: 'center',
            gap: 7,
            padding: compact ? '6px 10px 4px' : '8px 12px 5px',
            flexShrink: 0,
            borderBottom: `1px solid ${colors.borderSubtle}`,
            minWidth: 0,
          }}
          title={agentName}
        >
          {ambient && (
            <span
              aria-hidden
              title={badgeLabels[ambient]}
              style={{
                width: 7,
                height: 7,
                borderRadius: '50%',
                flexShrink: 0,
                background: badgeColors[ambient] ?? colors.mutedDim,
                animation:
                  ambient === 'thinking' || ambient === 'streaming' || ambient === 'background'
                    ? 'claudePulseDot 1.4s ease-in-out infinite'
                    : undefined,
              }}
            />
          )}
          <span
            style={{
              fontSize: '0.72rem',
              fontWeight: 700,
              color: colors.textBright,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              minWidth: 0,
            }}
          >
            {agentName}
          </span>
          {ambient && badgeLabels[ambient] && (
            <span style={{ fontSize: '0.62rem', color: colors.mutedDim, flexShrink: 0 }}>
              {badgeLabels[ambient]}
            </span>
          )}
        </div>
      )}

      {/* Tab strip: icon + label pills; live tabs carry a pulsing dot. */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 2,
          padding: compact ? '4px 6px' : '5px 7px',
          borderBottom: `1px solid ${colors.borderSubtle}`,
          flexShrink: 0,
          overflowX: 'auto',
          scrollbarWidth: 'none',
        }}
      >
        {tabs.map((t) => {
          const selected = tab === t.id;
          const TabIcon = t.icon;
          return (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              aria-label={t.label}
              title={t.label}
              style={{
                position: 'relative',
                fontSize: compact ? '0.66rem' : '0.7rem',
                fontWeight: 600,
                padding: compact ? '3px 7px' : '4px 9px',
                borderRadius: 'var(--wks-radius-pill, 99px)',
                border: `1px solid ${selected ? 'color-mix(in srgb, var(--wks-accent) 30%, transparent)' : 'transparent'}`,
                cursor: 'pointer',
                backgroundColor: selected ? 'var(--wks-accent-bg)' : 'transparent',
                color: selected ? colors.accent : colors.muted,
                display: 'inline-flex',
                alignItems: 'center',
                gap: 4,
                flexShrink: 0,
                fontFamily: 'inherit',
                transition: 'color 0.15s, background-color 0.15s, border-color 0.15s',
              }}
              onMouseEnter={(e) => {
                if (!selected) e.currentTarget.style.color = colors.text;
              }}
              onMouseLeave={(e) => {
                if (!selected) e.currentTarget.style.color = colors.muted;
              }}
            >
              <TabIcon size={compact ? 11 : 12} strokeWidth={2} aria-hidden />
              {/* Segmented control: only the active tab spends width on its
                  label — the rest stay icon+badge so all five fit the rail. */}
              {selected && t.label}
              {t.badge !== undefined && (
                <span
                  style={{
                    fontSize: '0.62rem',
                    padding: '0 5px',
                    borderRadius: 'var(--wks-radius-pill, 99px)',
                    backgroundColor: 'rgba(255,255,255,0.08)',
                    color: t.live ? colors.purple : selected ? colors.accent : colors.muted,
                    fontVariantNumeric: 'tabular-nums',
                    lineHeight: 1.5,
                  }}
                >
                  {t.badge}
                </span>
              )}
              {t.live && (
                <span
                  aria-hidden
                  style={{
                    width: 5,
                    height: 5,
                    borderRadius: '50%',
                    background: colors.purple,
                    animation: 'claudePulseDot 1.4s ease-in-out infinite',
                  }}
                />
              )}
            </button>
          );
        })}
        <div style={{ flex: 1 }} />
        {headerAccessory}
      </div>

      {/* Tab body — keyed by tab so switching gets the soft fade. */}
      <div
        key={tab}
        style={{
          flex: 1,
          overflowY: 'auto',
          padding: bodyPad,
          animation: 'claudeFadeIn 0.15s ease-out',
        }}
      >
        {tab === 'files' &&
          (files.length === 0 ? (
            <EmptyState icon={FileDiff} text="No files changed yet" />
          ) : (
            <>
              <div
                style={{
                  fontSize: '0.64rem',
                  color: colors.mutedDim,
                  margin: '0 0 6px 2px',
                  textTransform: 'uppercase',
                  letterSpacing: '0.05em',
                  fontWeight: 600,
                }}
              >
                {files.length} file{files.length === 1 ? '' : 's'} · click opens Review
              </div>
              {files.map((f, idx) => (
                <div
                  key={f.path}
                  title={`${f.path}\n\nClick to open in Review`}
                  onClick={() =>
                    requestReviewFile({ path: f.path, cwd: session?.liveCwd ?? session?.cwd })
                  }
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 7,
                    padding: '4px 5px',
                    margin: '0 -4px',
                    borderRadius: 6,
                    fontSize: '0.8rem',
                    borderTop: idx > 0 ? `1px solid ${colors.borderSubtle}` : undefined,
                    cursor: 'pointer',
                  }}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)';
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.backgroundColor = 'transparent';
                  }}
                >
                  {/* Op badge: + created (Write), ~ modified (Edit/patch). */}
                  <span
                    aria-hidden
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      width: 15,
                      height: 15,
                      borderRadius: 4,
                      flexShrink: 0,
                      fontSize: '0.7rem',
                      fontWeight: 700,
                      lineHeight: 1,
                      color: f.lastTool === 'Write' ? colors.success : colors.warning,
                      background: `color-mix(in srgb, ${
                        f.lastTool === 'Write' ? 'var(--wks-success)' : 'var(--wks-warning)'
                      } 12%, transparent)`,
                    }}
                  >
                    {f.lastTool === 'Write' ? '+' : '~'}
                  </span>
                  <span
                    style={{
                      color: colors.text,
                      fontFamily: 'var(--claude-mono-font, monospace)',
                      fontSize: '0.74rem',
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
                      fontSize: '0.66rem',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {f.dir}
                  </span>
                  <div style={{ flex: 1 }} />
                  {f.count > 1 && (
                    <span
                      title={`Touched ${f.count} times`}
                      style={{
                        color: colors.muted,
                        fontSize: '0.62rem',
                        flexShrink: 0,
                        background: 'rgba(255,255,255,0.06)',
                        borderRadius: 'var(--wks-radius-pill, 99px)',
                        padding: '0 5px',
                        fontVariantNumeric: 'tabular-nums',
                        lineHeight: 1.6,
                      }}
                    >
                      ×{f.count}
                    </span>
                  )}
                </div>
              ))}
            </>
          ))}

        {tab === 'plan' &&
          (!planStats ? (
            <EmptyState icon={ListChecks} text="No plan yet" />
          ) : (
            <>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  marginBottom: 10,
                }}
              >
                <span
                  style={{
                    fontSize: '0.66rem',
                    color: planStats.done === planStats.total ? colors.success : colors.accent,
                    fontWeight: 600,
                    textTransform: 'uppercase',
                    letterSpacing: '0.05em',
                    flexShrink: 0,
                    fontVariantNumeric: 'tabular-nums',
                  }}
                >
                  {planStats.done}/{planStats.total} done
                </span>
                {/* Plan progress meter — same-hue lighter track. */}
                <span
                  aria-hidden
                  style={{
                    flex: 1,
                    height: 4,
                    borderRadius: 2,
                    background: `color-mix(in srgb, ${
                      planStats.done === planStats.total
                        ? 'var(--wks-success)'
                        : 'var(--wks-accent)'
                    } 14%, transparent)`,
                    overflow: 'hidden',
                    display: 'inline-block',
                  }}
                >
                  <span
                    style={{
                      display: 'block',
                      width: `${planStats.total > 0 ? (planStats.done / planStats.total) * 100 : 0}%`,
                      height: '100%',
                      borderRadius: '2px 2px 2px 2px',
                      background:
                        planStats.done === planStats.total
                          ? 'var(--wks-success)'
                          : 'var(--wks-accent)',
                      transition: 'width 0.3s ease',
                    }}
                  />
                </span>
              </div>
              <PlanChecklist steps={plan!.steps} />
            </>
          ))}

        {tab === 'workflows' &&
          (workflows.length === 0 ? (
            <EmptyState icon={Workflow} text="No workflows running" />
          ) : (
            <>
              {liveWorkflows > 0 && (
                <div style={{ marginBottom: 8 }}>
                  <RunningMarker count={liveWorkflows} />
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
            <EmptyState icon={Bot} text="No subagents yet" />
          ) : (
            <>
              <div style={{ display: 'flex', alignItems: 'center', marginBottom: 8 }}>
                {liveSubagents > 0 && <RunningMarker count={liveSubagents} />}
                <div style={{ flex: 1 }} />
                <button
                  onClick={watchAgents}
                  title="Open all this session’s agents on one timeline"
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 4,
                    fontSize: '0.64rem',
                    fontWeight: 600,
                    padding: '2px 9px',
                    borderRadius: 'var(--wks-radius-pill, 99px)',
                    border: `1px solid ${colors.borderSubtle}`,
                    cursor: 'pointer',
                    backgroundColor: 'transparent',
                    color: colors.muted,
                    flexShrink: 0,
                    fontFamily: 'inherit',
                    transition: 'color 0.15s, border-color 0.15s',
                  }}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.borderColor =
                      'var(--wks-border-active, var(--wks-accent))';
                    e.currentTarget.style.color = colors.text;
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.borderColor = colors.borderSubtle;
                    e.currentTarget.style.color = colors.muted;
                  }}
                >
                  Monitor
                  <ArrowUpRight size={11} strokeWidth={2.2} aria-hidden />
                </button>
              </div>
              {subagents.map((sub) => (
                <SubagentRow key={sub.id} sub={sub} onOpen={() => watchSubagent(sub)} />
              ))}
            </>
          ))}

        {tab === 'usage' &&
          (!sl && !usage ? (
            <EmptyState icon={Gauge} text="No usage data yet" />
          ) : (
            <div>
              {model && (
                <div
                  style={{
                    fontSize: '0.78rem',
                    color: colors.textBright,
                    fontWeight: 650,
                    marginBottom: 10,
                  }}
                >
                  {model}
                </div>
              )}
              {sl?.capabilities &&
                (() => {
                  const c = sl.capabilities;
                  // A chip with a `focus` deep-links into the Context pane's
                  // matching section — what's actually taking up context space.
                  const chips: { label: string; focus?: string }[] = [];
                  if (c.fastMode) chips.push({ label: '⚡ Fast' });
                  if (c.apiKeySource && c.apiKeySource !== 'none')
                    chips.push({ label: `key: ${c.apiKeySource}` });
                  if (c.outputStyle && c.outputStyle !== 'default')
                    chips.push({ label: `style: ${c.outputStyle}` });
                  if (c.mcpServers) chips.push({ label: `${c.mcpServers} MCP`, focus: 'mcp' });
                  if (c.skills) chips.push({ label: `${c.skills} skills`, focus: 'skills' });
                  if (c.plugins) chips.push({ label: `${c.plugins} plugins`, focus: 'plugins' });
                  if (c.agents) chips.push({ label: `${c.agents} agents`, focus: 'agents' });
                  if (c.memoryFiles)
                    chips.push({ label: `${c.memoryFiles} memory`, focus: 'memory' });
                  if (!chips.length) return null;
                  const openContext = (focus?: string) =>
                    session &&
                    requestContextPane({
                      sessionId: session.sessionId,
                      agentName,
                      focus,
                    });
                  const chipStyle: React.CSSProperties = {
                    fontSize: '0.64rem',
                    color: colors.muted,
                    background: 'rgba(255,255,255,0.04)',
                    border: `1px solid ${colors.borderSubtle}`,
                    borderRadius: 'var(--wks-radius-pill, 99px)',
                    padding: '1px 8px',
                    whiteSpace: 'nowrap',
                    fontFamily: 'inherit',
                  };
                  return (
                    <div
                      style={{
                        display: 'flex',
                        flexWrap: 'wrap',
                        gap: 4,
                        marginBottom: 12,
                      }}
                    >
                      {chips.map((chip) =>
                        chip.focus ? (
                          <button
                            key={chip.label}
                            onClick={() => openContext(chip.focus)}
                            title="See what's taking up context space"
                            style={{ ...chipStyle, cursor: 'pointer' }}
                            onMouseEnter={(e) => {
                              e.currentTarget.style.borderColor =
                                'var(--wks-border-active, var(--wks-accent))';
                              e.currentTarget.style.color = colors.text;
                            }}
                            onMouseLeave={(e) => {
                              e.currentTarget.style.borderColor = colors.borderSubtle;
                              e.currentTarget.style.color = colors.muted;
                            }}
                          >
                            {chip.label}
                          </button>
                        ) : (
                          <span key={chip.label} style={chipStyle}>
                            {chip.label}
                          </span>
                        ),
                      )}
                    </div>
                  );
                })()}
              {ctxPct !== undefined && (
                <div
                  role="button"
                  title="See what's taking up context space"
                  onClick={() =>
                    session && requestContextPane({ sessionId: session.sessionId, agentName })
                  }
                  style={{ cursor: 'pointer' }}
                >
                  <UsageBar label="Context window" pct={ctxPct} />
                </div>
              )}
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
                <div style={{ fontSize: '0.72rem', color: colors.warning, marginBottom: 8 }}>
                  ⚠ {sl.rateLimitWarning}
                </div>
              )}
              {sl?.overageOutOfCredits && (
                <div style={{ fontSize: '0.72rem', color: colors.muted, marginBottom: 8 }}>
                  Monthly overage: out of credits
                </div>
              )}

              {/* Session totals: 2×2 stat tiles, then the row-level extras. */}
              {(inTok !== undefined || outTok !== undefined || cost !== undefined || session) && (
                <div
                  style={{
                    display: 'grid',
                    gridTemplateColumns: '1fr 1fr',
                    gap: 6,
                    marginTop: 12,
                  }}
                >
                  {inTok !== undefined && (
                    <StatTile label="Input tokens" value={fmtTokens(inTok) || '0'} />
                  )}
                  {outTok !== undefined && (
                    <StatTile label="Output tokens" value={fmtTokens(outTok) || '0'} />
                  )}
                  {cost !== undefined && Number.isFinite(cost) && (
                    <StatTile label="Cost" value={`$${cost.toFixed(2)}`} />
                  )}
                  {session && (
                    <StatTile label="Tool calls" value={String(session.totalToolCalls)} />
                  )}
                </div>
              )}
              <div style={{ marginTop: 10, fontSize: '0.72rem', lineHeight: 1.6 }}>
                {session && (session.compactionCount ?? 0) > 0 && (
                  <div
                    title="Context compactions this session (frequent = context churn)"
                    style={statRowStyle}
                  >
                    <span style={{ color: colors.muted }}>Compactions</span>
                    <span style={{ color: colors.text, fontVariantNumeric: 'tabular-nums' }}>
                      {session.compacting
                        ? `${session.compactionCount} · now`
                        : session.compactionCount}
                    </span>
                  </div>
                )}
                {session && <BudgetRow sessionId={session.sessionId} cost={cost} />}
              </div>
            </div>
          ))}
      </div>
    </div>
  );
};

/** Label-over-value tile for the usage totals grid. */
const StatTile: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <div
    style={{
      padding: '7px 10px',
      borderRadius: 8,
      border: `1px solid ${colors.borderSubtle}`,
      background: 'rgba(255,255,255,0.025)',
      minWidth: 0,
    }}
  >
    <div
      style={{
        fontSize: '0.6rem',
        color: colors.mutedDim,
        textTransform: 'uppercase',
        letterSpacing: '0.05em',
        fontWeight: 600,
        marginBottom: 2,
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
      }}
    >
      {label}
    </div>
    <div
      style={{
        fontSize: '0.82rem',
        color: colors.textBright,
        fontWeight: 650,
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
      }}
    >
      {value}
    </div>
  </div>
);

/** Shared spaced row for the list-style stats under the tiles. */
const statRowStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  padding: '3px 0',
};

export default InspectorCard;
