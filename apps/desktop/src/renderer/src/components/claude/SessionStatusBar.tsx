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
 * Claude Code's terminal status line. Rendered in the agent pane's bottom
 * status bar (next to the status badge).
 *
 * Division of labor with ComposerControls (which always sits beside/above this
 * bar): the pills own the *controls* — model, effort, permission mode — and
 * this bar owns the *telemetry* — dir/branch, plan, context, tokens/cost and
 * the account rate-limit windows. Nothing appears in both.
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
      height: 14,
      flexShrink: 0,
      background: 'var(--wks-border)',
      opacity: 0.5,
    }}
  />
);

/** Thin rounded meter — the exact track treatment the sidebar and agent cards
 *  use for their context bars (subtle-border track, smooth threshold-colored
 *  fill), so every gauge in the app reads as one family. The 2% floor keeps a
 *  sliver of fill visible for tiny non-zero values. */
const Track: React.FC<{ pct: number; color: string; width?: number }> = ({
  pct,
  color,
  width = 40,
}) => {
  const clamped = Math.max(0, Math.min(100, pct));
  return (
    <span
      style={{
        width,
        height: 4,
        borderRadius: 999,
        flexShrink: 0,
        background: 'var(--wks-border-subtle)',
        overflow: 'hidden',
        display: 'inline-block',
      }}
    >
      <span
        style={{
          display: 'block',
          height: '100%',
          width: `${clamped > 0 ? Math.max(2, clamped) : 0}%`,
          borderRadius: 999,
          background: color,
        }}
      />
    </span>
  );
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

  // (Permission mode is deliberately NOT shown here — the ComposerControls
  // pills beside/above this bar own model + effort + permission mode, and
  // repeating the mode made the two rows read as duplicates.)

  // Plan progress: `plan 3/7`, ticks + the current step's activeForm as tooltip.
  // Hidden when there's no plan (simplest rule — a finished plan still reads as
  // a useful "all done" until the next turn clears it).
  const plan = planProgress(snapshot?.plan);

  // (The live-subagent count lives in the ClaudePane toolbar alongside this
  // bar — kept there so the number isn't shown twice.)

  // Until the first reading arrives, render nothing so the toolbar stays clean.
  const hasAny =
    (showModel && model) ||
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
            fontSize: '0.64rem',
            fontWeight: 700,
            padding: '1px 6px',
            borderRadius: 'var(--wks-radius-pill)',
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
      {plan && (
        <>
          <Sep />
          <span
            title={plan.active?.activeForm ?? plan.active?.content ?? 'Plan progress'}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 5,
              color: plan.done >= plan.total ? 'var(--wks-success)' : 'var(--wks-accent-text)',
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            <span style={{ color: 'var(--wks-text-muted)' }}>plan</span>
            <Track
              pct={(plan.done / plan.total) * 100}
              color={plan.done >= plan.total ? 'var(--wks-success)' : 'var(--wks-accent-text)'}
              width={32}
            />
            {plan.done}/{plan.total}
          </span>
        </>
      )}
      {snapshot?.compacting && (
        <>
          <Sep />
          <span
            title="Claude is compacting its context window"
            style={{ color: 'var(--wks-warning)', fontVariantNumeric: 'tabular-nums' }}
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
            <Track pct={ctxPct} color={ctxColor(ctxPct)} />
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
            {/* Value-first, unit muted — the same "142k tok · $0.42" phrasing
                the sidebar tooltip and agent cards use. */}
            {tokens !== undefined && (
              <span>
                <span style={{ color: 'var(--wks-text-secondary)' }}>{fmtTokens(tokens)}</span>
                <span style={{ color: 'var(--wks-text-muted)' }}> tok</span>
              </span>
            )}
            {tokens !== undefined && cost !== undefined && (
              <span style={{ color: 'var(--wks-text-faint)' }}>·</span>
            )}
            {cost !== undefined && (
              <span style={{ color: 'var(--wks-text-secondary)' }}>{fmtUSD(cost)}</span>
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
          {
            label: 'Mo',
            name: 'Monthly window',
            pct: monthly,
            reset: monthlyReset,
            at: monthlyResetsAt,
          },
        ].filter((w) => w.pct !== undefined || w.at !== undefined);
        if (!windows.length) return null;
        return (
          <>
            <Sep />
            <span
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 9,
                color: 'var(--wks-text-secondary)',
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {windows.map((w) => {
                const at = fmtResetAt(w.at);
                const tip = [
                  w.name,
                  w.pct !== undefined ? `${Math.round(w.pct)}% used` : undefined,
                  at ? `resets ${at}` : undefined,
                ]
                  .filter(Boolean)
                  .join(' · ');
                return (
                  <span
                    key={w.label}
                    title={tip}
                    style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}
                  >
                    <span style={{ color: 'var(--wks-text-muted)' }}>{w.label}</span>
                    {w.pct !== undefined ? (
                      <>
                        <Track pct={w.pct} color={ctxColor(w.pct)} width={26} />
                        {Math.round(w.pct)}%
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
