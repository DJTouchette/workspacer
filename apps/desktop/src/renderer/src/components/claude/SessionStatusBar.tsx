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
 * Git branch comes from a lightweight `gitStatus` poll against the agent's
 * effective cwd (`snapshot.liveCwd` — the worktree the agent entered — falling
 * back to the spawn cwd). When the agent works in a worktree, a chip marks it.
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
  // Follow the agent: liveCwd is set only while it works somewhere other than
  // the spawn dir (a git worktree), so its presence doubles as the indicator.
  const activeCwd = snapshot?.liveCwd || cwd || snapshot?.cwd;
  const inWorktree = !!snapshot?.liveCwd;
  const dir = baseName(activeCwd);

  // Branch of the effective cwd — fetched on cwd change and on a slow clock
  // (branches move on checkout/commit, not per-keystroke). Best-effort: not a
  // repo / no bridge → no branch segment.
  const [branch, setBranch] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!activeCwd) {
      setBranch(null);
      return;
    }
    let live = true;
    const fetchBranch = () => {
      try {
        window.electronAPI
          .gitStatus?.(activeCwd)
          ?.then((s) => {
            if (live) setBranch(s.branch);
          })
          .catch(() => {
            if (live) setBranch(null);
          });
      } catch {
        // web polyfill / test mocks without gitStatus
      }
    };
    fetchBranch();
    const t = setInterval(fetchBranch, 60_000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [activeCwd]);
  const {
    model,
    ctxPct,
    tokens,
    costUSD: cost,
    fiveHourPct: five,
    fiveHourResetsAt,
    sevenDayPct: seven,
    sevenDayResetsAt,
    monthlyPct: monthly,
    monthlyResetsAt,
  } = deriveSessionStats(snapshot);
  // The reset countdowns are computed at render time; between statusLine ticks
  // (e.g. an idle session) they'd freeze, so tick a re-render once a minute
  // while any reset timestamp is on display.
  const [, bumpClock] = React.useReducer((n: number) => n + 1, 0);
  const hasResets =
    fiveHourResetsAt !== undefined ||
    sevenDayResetsAt !== undefined ||
    monthlyResetsAt !== undefined;
  React.useEffect(() => {
    if (!hasResets) return;
    const timer = setInterval(bumpClock, 60_000);
    return () => clearInterval(timer);
  }, [hasResets]);
  const fiveReset = fmtResetIn(fiveHourResetsAt);
  const sevenReset = fmtResetIn(sevenDayResetsAt);
  const monthlyReset = fmtResetIn(monthlyResetsAt);

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
    snapshot?.compacting ||
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
      {dir && (
        <span title={activeCwd} style={{ color: 'var(--wks-accent-text)', fontWeight: 600 }}>
          {dir}
        </span>
      )}
      {branch && (
        <span
          title={inWorktree ? `On ${branch} — worktree at ${activeCwd}` : `On ${branch}`}
          style={{ color: 'var(--wks-text-secondary)' }}
        >
          {'⎇'} {branch}
        </span>
      )}
      {inWorktree && (
        <span
          title={`Agent is working in a git worktree: ${activeCwd}`}
          style={{
            fontSize: '0.6rem',
            fontWeight: 700,
            padding: '1px 6px',
            borderRadius: 'var(--wks-radius-pill, 999px)',
            letterSpacing: '0.04em',
            color: 'var(--wks-accent-text)',
            border: '1px solid color-mix(in srgb, var(--wks-accent) 45%, transparent)',
            background: 'color-mix(in srgb, var(--wks-accent) 12%, transparent)',
            flexShrink: 0,
          }}
        >
          worktree
        </span>
      )}
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
      {snapshot?.compacting && (
        <>
          <Sep />
          <span
            title="Claude is compacting its context window"
            style={{ color: 'var(--wks-warn, #d29922)', fontVariantNumeric: 'tabular-nums' }}
          >
            compacting…
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
      {(() => {
        // Render each account window when Claude gives us EITHER a utilization %
        // OR just a reset time (many accounts only report the reset while
        // comfortably within a window — see the rate-limit plumbing notes).
        const windows = [
          { label: '5h', name: '5h window', pct: five, reset: fiveReset, at: fiveHourResetsAt },
          { label: '7d', name: '7d window', pct: seven, reset: sevenReset, at: sevenDayResetsAt },
          { label: 'Mo', name: 'Monthly window', pct: monthly, reset: monthlyReset, at: monthlyResetsAt },
        ].filter((w) => w.pct !== undefined || w.at !== undefined);
        if (!windows.length) return null;
        return (
          <>
            <Sep />
            <span style={{ color: 'var(--wks-text-secondary)', fontVariantNumeric: 'tabular-nums' }}>
              {windows.map((w, i) => {
                const at = fmtResetAt(w.at);
                return (
                  <span key={w.label} title={at ? `${w.name} resets ${at}` : undefined}>
                    {i > 0 ? ' ' : ''}
                    <span style={{ color: 'var(--wks-text-muted)' }}>{w.label} </span>
                    {w.pct !== undefined ? (
                      <>
                        {Math.round(w.pct)}%
                        {w.reset && (
                          <span style={{ color: 'var(--wks-text-muted)' }}>·{w.reset}</span>
                        )}
                      </>
                    ) : (
                      <span style={{ color: 'var(--wks-text-muted)' }}>
                        {w.reset ? `resets ${w.reset}` : 'ok'}
                      </span>
                    )}
                  </span>
                );
              })}
            </span>
          </>
        );
      })()}
    </span>
  );
};

export default SessionStatusBar;
