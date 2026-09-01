import React from 'react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import {
  deriveSessionStats,
  usageWindows,
  planProgress,
  fmtTokens,
  fmtUSD,
  fmtResetAt,
  fmtResetIn,
  fmtWindowLength,
  ctxColor,
  withRecordedUsage,
} from '../../lib/sessionStats';
import { useRecordedUsage } from '../../contexts/RecordedUsageContext';
import { IconModel } from '../wksIcons';
import { HubChip } from '../HubChip';
import { ConfigContext } from '../../contexts/ConfigContext';
import { UsageDetailDialog } from './UsageDetailDialog';

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
 * Deliberately sparse: identity (dir/branch), plan N/M, the ctx meter (the
 * bar's ONE gauge) and cost. Tokens live in the cost tooltip; the account
 * rate-limit windows are warning-only — account-scoped data that's identical
 * in every pane, fully charted on the Overview pane, so here a window appears
 * only once it's genuinely close to the limit.
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

/**
 * Default utilization at which an account window earns its OWN chip in the bar.
 *
 * The windows are account-scoped: identical in every open pane and fully charted
 * on the Overview pane, so listing all of them in every agent's toolbar is pure
 * duplication. But hiding them entirely below the threshold left the bar silent
 * about usage most of the time, which read as "workspacer only knows the 5-hour
 * window". So the bar always shows the single busiest window as one chip, adds
 * the others once they are genuinely close to a limit, and makes the whole group
 * open the usage dialog, which lists every window with its length and reset.
 *
 * Overridable per install with `ui.usageWindowChipPct` (0 keeps every reported
 * window in the bar).
 */
const WINDOW_WARN_PCT = 70;

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
  /** The session this bar belongs to, for surfaces that render it BEFORE a live
   *  snapshot exists (a cold-start pane). Without it the bar has no key to look
   *  up the history DB's recorded cost/tokens with, because the only id it
   *  otherwise has lives inside the snapshot it doesn't have. */
  sessionId?: string;
  cwd?: string;
  /** Render the model segment. Off by default: in the agent pane the model is
   *  already shown by ComposerControls sitting right beside/above this bar, so
   *  showing it here too would duplicate it. Surfaces that render this bar
   *  without a nearby ComposerControls (e.g. inspector/fleet cards) opt in. */
  showModel?: boolean;
}

export const SessionStatusBar: React.FC<Props> = ({
  snapshot,
  sessionId,
  cwd,
  showModel = false,
}) => {
  // Follow the agent: liveCwd is set only while it works somewhere other than
  // the spawn dir (a git worktree), so its presence doubles as the indicator.
  const activeCwd = snapshot?.liveCwd || cwd || snapshot?.cwd;
  const inWorktree = !!snapshot?.liveCwd;
  const dir = baseName(activeCwd);

  // Branch of the effective cwd — fetched on cwd change and on a slow clock
  // (branches move on checkout/commit, not per-keystroke). Best-effort: not a
  // repo / no bridge → no branch segment. A remote (federated) session's cwd
  // names the PEER's filesystem — running local git against it can only fail
  // (and log-spam), so the segment is simply omitted.
  const remote = !!snapshot?.hub;
  const [branch, setBranch] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!activeCwd || remote) {
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
  }, [activeCwd, remote]);
  // Live figures first; the history DB's last-recorded ones fill the gaps a
  // cold start leaves (no snapshot ⇒ no cost, no tokens, no bar at all).
  const recorded = useRecordedUsage(snapshot?.sessionId ?? sessionId);
  const stats = withRecordedUsage(deriveSessionStats(snapshot), recorded);
  const { model, ctxPct, billedTokens, costUSD: cost } = stats;
  // Every window the provider actually reported, in order. Empty for a session
  // whose provider sends none, where the group renders nothing at all.
  const windows = usageWindows(stats);
  // Read the context directly rather than the throwing useConfig hook: this bar
  // also renders inside inspector and fleet cards, which are mounted in isolated
  // embeds and tests without a ConfigProvider. No provider means the default.
  const chipPct =
    React.useContext(ConfigContext)?.config?.ui?.usageWindowChipPct ?? WINDOW_WARN_PCT;
  const [usageOpen, setUsageOpen] = React.useState(false);
  // (No re-render clock needed: the only time-based text left is the absolute
  // reset time inside a warning window's tooltip, computed on hover-render.)

  // (Permission mode is deliberately NOT shown here — the ComposerControls
  // pills beside/above this bar own model + effort + permission mode, and
  // repeating the mode made the two rows read as duplicates.)

  // Plan progress: `plan 3/7`, ticks + the current step's activeForm as tooltip.
  // Hidden when there's no plan (simplest rule — a finished plan still reads as
  // a useful "all done" until the next turn clears it).
  const plan = planProgress(snapshot?.plan);
  /** Appended to the cost/token tooltips when the figure is the last RECORDED
   *  one rather than a live reading (see withRecordedUsage). */
  const recordedSuffix = stats.recorded
    ? '\n\nLast recorded for this session — not a live reading.'
    : '';

  // (The live-subagent count lives in the ClaudePane toolbar alongside this
  // bar — kept there so the number isn't shown twice.)

  // Until the first reading arrives, render nothing so the toolbar stays clean.
  const hasAny =
    (showModel && model) ||
    plan ||
    snapshot?.compacting ||
    ctxPct !== undefined ||
    billedTokens !== undefined ||
    cost !== undefined ||
    windows.length > 0;
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
      {/* Federation: the workspace's persistent "this agent runs on peer X"
          marker (the sidebar card carries the same chip). Offline peer link
          flips it to the warning tone. */}
      {snapshot?.hub && <HubChip name={snapshot.hub} offline={!!snapshot.hubOffline} />}
      {dir && (
        <span title={activeCwd} style={{ color: 'var(--wks-accent-text)', fontWeight: 600 }}>
          {dir}
        </span>
      )}
      {branch && (
        // Worktree signal rides on the branch itself (accent tint) instead of
        // a separate chip — same information, one fewer pill.
        <span
          title={
            inWorktree ? `On ${branch} — isolated git worktree at ${activeCwd}` : `On ${branch}`
          }
          style={{
            color: inWorktree ? 'var(--wks-accent-text)' : 'var(--wks-text-secondary)',
          }}
        >
          {'⎇'} {branch}
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
            data-testid="session-status-context-bar"
            aria-label={`Active context ${Math.round(ctxPct)}% of runtime-confirmed window`}
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
      {(billedTokens !== undefined || cost !== undefined) && (
        <>
          <Sep />
          {/* Cost is the glanceable number; BILLED tokens ride in its tooltip.
              This is the cost side, not occupancy — every turn re-sends the
              whole conversation, so it climbs into the tens of millions on a
              long worker while the `ctx` meter to its left stays put. The
              tooltip and the standalone label both say "billed" for that
              reason; a bare "30M tok" here read as context and was reported as
              a bug three times over. */}
          {cost !== undefined ? (
            <span
              title={
                (billedTokens !== undefined
                  ? `${fmtTokens(billedTokens)} tokens billed this session (cumulative — every turn re-sends the conversation, so this is not the context window)`
                  : '') + recordedSuffix
              }
              style={{ color: 'var(--wks-text-secondary)', fontVariantNumeric: 'tabular-nums' }}
            >
              {fmtUSD(cost)}
            </span>
          ) : (
            <span
              title={`${fmtTokens(billedTokens!)} tokens billed this session (cumulative — not the context window)${recordedSuffix}`}
              style={{ fontVariantNumeric: 'tabular-nums' }}
            >
              <span style={{ color: 'var(--wks-text-secondary)' }}>{fmtTokens(billedTokens!)}</span>
              <span style={{ color: 'var(--wks-text-muted)' }}> billed</span>
            </span>
          )}
          {/* Cold start: the figures came from the history DB's last snapshot,
              not from a live session. Say so inline — a bare number here would
              read as this session spending right now. */}
          {stats.recorded && (
            <span style={{ color: 'var(--wks-text-disabled)', fontSize: '0.9em' }}>
              last recorded
            </span>
          )}
        </>
      )}
      {(() => {
        // Which windows earn a chip: every one at or above the threshold, and
        // failing that the busiest single window, so the bar is never silent
        // about usage while the detail is one click away. A window with a reset
        // time but no percentage can't be ranked or coloured, so it stays in the
        // dialog only.
        const measured = windows.filter((w) => w.pct !== undefined);
        const hot = measured.filter((w) => w.pct! >= chipPct);
        const shown = hot.length
          ? hot
          : measured.length
            ? [measured.reduce((a, b) => (b.pct! > a.pct! ? b : a))]
            : [];
        if (!shown.length && !windows.length) return null;
        return (
          <>
            <Sep />
            <span
              role="button"
              tabIndex={0}
              aria-label="Show usage detail"
              onClick={() => setUsageOpen(true)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  setUsageOpen(true);
                }
              }}
              title="Usage and account limits. Click for detail."
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 9,
                fontVariantNumeric: 'tabular-nums',
                cursor: 'pointer',
              }}
            >
              {shown.length === 0 ? (
                <span style={{ color: 'var(--wks-text-muted)' }}>limits</span>
              ) : (
                shown.map((w) => {
                  const at = fmtResetAt(w.resetsAt);
                  const inS = fmtResetIn(w.resetsAt);
                  const length = fmtWindowLength(w.windowMins);
                  const tip = [
                    length ? `${w.label} (${length} window)` : w.label,
                    `${Math.round(w.pct!)}% used`,
                    inS ? `resets in ${inS}` : undefined,
                    at,
                  ]
                    .filter(Boolean)
                    .join(' · ');
                  return (
                    <span
                      key={w.key}
                      title={tip}
                      style={{
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 4,
                        color: ctxColor(w.pct!),
                      }}
                    >
                      <span style={{ color: 'var(--wks-text-muted)' }}>{w.short}</span>
                      {Math.round(w.pct!)}%
                    </span>
                  );
                })
              )}
            </span>
          </>
        );
      })()}
      {usageOpen && <UsageDetailDialog snapshot={snapshot} onClose={() => setUsageOpen(false)} />}
    </span>
  );
};

export default SessionStatusBar;
