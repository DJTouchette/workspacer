import React from 'react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import {
  deriveSessionStats,
  planProgress,
  fmtTokens,
  fmtUSD,
  fmtResetIn,
  fmtResetAt,
  ctxColor,
} from '../../lib/sessionStats';
import { IconModel } from '../wksIcons';

/**
 * A compact, single-line status readout — Workspacer's in-app equivalent of
 * Claude Code's terminal status line. Rendered inline in the agent pane's top
 * toolbar (next to the status badge).
 *
 * Data comes from `snapshot.statusLine` (fed by claudemon's /statusline/stream,
 * the only channel carrying Claude's authoritative context-%, cost, and the
 * 5h/7d rate-limit windows). Where the statusLine hasn't arrived yet we fall
 * back to the transcript-derived `usage` so it isn't blank.
 *
 * Note: git branch isn't shown — it lives only in Claude's statusLine *script*
 * (which shells out to git), not in the JSON channel we ingest.
 */

function baseName(p: string | undefined): string {
  if (!p) return '';
  return (
    p
      .replace(/[/\\]+$/, '')
      .split(/[/\\]/)
      .pop() || p
  );
}

/** A thin vertical rule between HUD groups, replacing the ASCII pipe. */
const Sep: React.FC = () => (
  <span
    style={{
      width: 1,
      height: 17,
      flexShrink: 0,
      background: 'var(--wks-border, #555)',
      opacity: 0.7,
    }}
  />
);

/** Segmented 10-tick context gauge — filled ticks take the threshold color. */
const CtxBar: React.FC<{ pct: number }> = ({ pct }) => {
  const filled = Math.max(0, Math.min(10, Math.round((pct / 100) * 10)));
  const color = ctxColor(pct);
  return (
    <span style={{ display: 'inline-flex', gap: 2, alignItems: 'center' }}>
      {Array.from({ length: 10 }, (_, i) => (
        <span
          key={i}
          style={{
            width: 5,
            height: 13,
            borderRadius: 1,
            background: i < filled ? color : 'var(--wks-bg-elevated, #444)',
          }}
        />
      ))}
    </span>
  );
};

/** Compact plan gauge — one tick per step (capped), filled by completion. All
 *  done tints green, otherwise accent. Mirrors CtxBar's segmented-ticks look. */
const PlanTicks: React.FC<{ done: number; total: number }> = ({ done, total }) => {
  const ticks = Math.min(total, 10);
  const filled = Math.round((done / total) * ticks);
  const color = done >= total ? 'var(--wks-success, #3fb950)' : 'var(--wks-accent-text)';
  return (
    <span style={{ display: 'inline-flex', gap: 2, alignItems: 'center' }}>
      {Array.from({ length: ticks }, (_, i) => (
        <span
          key={i}
          style={{
            width: 4,
            height: 13,
            borderRadius: 1,
            background: i < filled ? color : 'var(--wks-bg-elevated, #444)',
          }}
        />
      ))}
    </span>
  );
};

/** Compact label + color per permission-mode id. Claude ids come from hook
 *  `permission_mode`; 'ask'/'yolo' are the managed-provider (codex/opencode/pi)
 *  spawn settings. Unknown ids fall back to the raw id in the default color. */
const MODE_DISPLAY: Record<string, { label: string; color: string }> = {
  default: { label: 'ask', color: 'var(--wks-text-secondary)' },
  plan: { label: 'plan', color: '#38bdf8' },
  acceptEdits: { label: 'accept edits', color: 'var(--wks-warning)' },
  auto: { label: 'auto', color: 'var(--wks-success)' },
  dontAsk: { label: "don't ask", color: 'var(--wks-warning)' },
  bypassPermissions: { label: 'bypass', color: 'var(--wks-error)' },
  ask: { label: 'ask', color: 'var(--wks-text-secondary)' },
  yolo: { label: 'full access', color: 'var(--wks-error)' },
};

interface Props {
  snapshot?: ClaudeSessionSnapshot | null;
  cwd?: string;
  /** Render the model segment. Off by default: in the agent pane the model is
   *  already shown by ComposerControls sitting right beside/above this bar, so
   *  showing it here too would duplicate it. Surfaces that render this bar
   *  without a nearby ComposerControls (e.g. inspector/fleet cards) opt in. */
  showModel?: boolean;
}

export const SessionStatusBar: React.FC<Props> = ({ snapshot, cwd, showModel = false }) => {
  const dir = baseName(cwd || snapshot?.cwd);
  const {
    model,
    ctxPct,
    tokens,
    costUSD: cost,
    fiveHourPct: five,
    fiveHourResetsAt,
    sevenDayPct: seven,
    sevenDayResetsAt,
  } = deriveSessionStats(snapshot);
  // The reset countdowns are computed at render time; between statusLine ticks
  // (e.g. an idle session) they'd freeze, so tick a re-render once a minute
  // while any reset timestamp is on display.
  const [, bumpClock] = React.useReducer((n: number) => n + 1, 0);
  const hasResets = fiveHourResetsAt !== undefined || sevenDayResetsAt !== undefined;
  React.useEffect(() => {
    if (!hasResets) return;
    const timer = setInterval(bumpClock, 60_000);
    return () => clearInterval(timer);
  }, [hasResets]);
  const fiveReset = fmtResetIn(fiveHourResetsAt);
  const sevenReset = fmtResetIn(sevenDayResetsAt);

  // Permission mode: live hook telemetry (follows shift+tab cycling in the
  // TUI) wins over the requested-at-spawn setting. Managed providers never
  // send the live value, so they show their spawn setting.
  const modeId = snapshot?.livePermissionMode ?? snapshot?.settings?.permissionMode;
  const mode = modeId
    ? (MODE_DISPLAY[modeId] ?? { label: modeId, color: 'var(--wks-text-secondary)' })
    : undefined;

  // Plan progress: `plan 3/7`, ticks + the current step's activeForm as tooltip.
  // Hidden when there's no plan (simplest rule — a finished plan still reads as
  // a useful "all done" until the next turn clears it).
  const plan = planProgress(snapshot?.plan);

  // (The live-subagent count lives in the ClaudePane toolbar alongside this
  // bar — kept there so the number isn't shown twice.)

  // Until the first reading arrives, render nothing so the toolbar stays clean.
  const hasAny =
    (showModel && model) ||
    mode ||
    plan ||
    ctxPct !== undefined ||
    tokens !== undefined ||
    cost !== undefined;
  if (!hasAny) return null;

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 8,
        minWidth: 0,
        overflow: 'hidden',
        fontFamily: 'var(--wks-font-mono, var(--claude-mono-font, monospace))',
        fontSize: '0.78rem',
        whiteSpace: 'nowrap',
      }}
    >
      {dir && <span style={{ color: 'var(--wks-accent-text)', fontWeight: 600 }}>{dir}</span>}
      {showModel && model && (
        <>
          <Sep />
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              color: 'var(--wks-text-secondary)',
            }}
          >
            <IconModel size={14} strokeWidth={2} accent="currentColor" />
            {model}
          </span>
        </>
      )}
      {mode && (
        <>
          <Sep />
          <span
            title="Permission mode — cycles live with shift+tab in the terminal view"
            style={{ color: mode.color, fontWeight: 600 }}
          >
            {mode.label}
          </span>
        </>
      )}
      {plan && (
        <>
          <Sep />
          <span
            title={plan.active?.activeForm ?? plan.active?.content ?? 'Plan progress'}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 5,
              color:
                plan.done >= plan.total ? 'var(--wks-success, #3fb950)' : 'var(--wks-accent-text)',
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            <span style={{ color: 'var(--wks-text-muted)' }}>plan</span>
            <PlanTicks done={plan.done} total={plan.total} />
            {plan.done}/{plan.total}
          </span>
        </>
      )}
      {ctxPct !== undefined && (
        <>
          <Sep />
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 5,
              color: ctxColor(ctxPct),
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            <span style={{ color: 'var(--wks-text-muted)' }}>ctx</span>
            <CtxBar pct={ctxPct} />
            {Math.round(ctxPct)}%
          </span>
        </>
      )}
      {(tokens !== undefined || cost !== undefined) && (
        <>
          <Sep />
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 5,
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            {tokens !== undefined && (
              <span>
                <span style={{ color: 'var(--wks-text-muted)' }}>tok </span>
                <span style={{ color: 'var(--wks-text-secondary)' }}>{fmtTokens(tokens)}</span>
              </span>
            )}
            {cost !== undefined && (
              <span style={{ color: 'var(--wks-accent-text)' }}>{fmtUSD(cost)}</span>
            )}
          </span>
        </>
      )}
      {(five !== undefined || seven !== undefined) && (
        <>
          <Sep />
          <span style={{ color: 'var(--wks-text-secondary)', fontVariantNumeric: 'tabular-nums' }}>
            {five !== undefined && (
              <span
                title={
                  fmtResetAt(fiveHourResetsAt) && `5h window resets ${fmtResetAt(fiveHourResetsAt)}`
                }
              >
                <span style={{ color: 'var(--wks-text-muted)' }}>5h </span>
                {Math.round(five)}%
                {fiveReset && <span style={{ color: 'var(--wks-text-muted)' }}>·{fiveReset}</span>}
              </span>
            )}
            {seven !== undefined && (
              <span
                title={
                  fmtResetAt(sevenDayResetsAt) && `7d window resets ${fmtResetAt(sevenDayResetsAt)}`
                }
              >
                {' '}
                <span style={{ color: 'var(--wks-text-muted)' }}>7d </span>
                {Math.round(seven)}%
                {sevenReset && (
                  <span style={{ color: 'var(--wks-text-muted)' }}>·{sevenReset}</span>
                )}
              </span>
            )}
          </span>
        </>
      )}
    </span>
  );
};

export default SessionStatusBar;
