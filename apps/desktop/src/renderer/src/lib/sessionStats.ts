import type { ClaudeSessionSnapshot, SessionStatusLine, SessionUsage } from '../types/claudeSession';

/**
 * The single source of truth for "what numbers do we show for this session".
 *
 * Claude's own statusLine (`/statusline/stream`) carries the authoritative
 * context-%, cost, cumulative tokens and 5h/7d rate-limit windows. The
 * transcript-derived `SessionUsage` is an approximation we compute ourselves —
 * useful as a fallback before the first statusLine reading arrives, but it can
 * disagree with Claude's number.
 *
 * Both the agent pane's status bar (`SessionStatusBar`) and the sidebar's
 * per-agent context bar derive from this so they can never show different
 * context percentages for the same session.
 */
// ── Shared formatters + thresholds ───────────────────────────────────────────
//
// These were re-declared (with subtly different thresholds) in SideBar,
// FleetDeck, AgentCard, and SessionStatusBar — so the same context % could show
// amber on one surface and green on another, and token counts formatted
// differently. Single definitions here keep every agent surface consistent.

/** 142345 → "142k", 1_200_000 → "1.2M", ≥10M → "12M" (drops the decimal). */
export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return `${n}`;
}

/** Cost, compact: `$1.23`, `<$0.01` for tiny non-zero, `$0.00` for nothing. */
export function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}

/** Context-window fill color by PERCENT (0–100): green → amber (≥70) → red (≥90). */
export function ctxColor(pct: number): string {
  if (pct >= 90) return 'var(--wks-danger, #e05555)';
  if (pct >= 70) return 'var(--wks-warning, #e0a000)';
  return 'var(--wks-success, #3fb950)';
}

export interface DerivedSessionStats {
  model?: string;
  /** Context window fill, 0–100. */
  ctxPct?: number;
  /** Cumulative input+output tokens. */
  tokens?: number;
  costUSD?: number;
  fiveHourPct?: number;
  sevenDayPct?: number;
}

export function deriveSessionStats(
  snapshot?: Pick<ClaudeSessionSnapshot, 'usage' | 'statusLine'> | null,
): DerivedSessionStats {
  const sl: SessionStatusLine | undefined = snapshot?.statusLine;
  const usage: SessionUsage | null | undefined = snapshot?.usage;

  const model = sl?.modelDisplay ?? (usage?.model ? usage.model.replace(/^claude-/, '') : undefined);

  // Context %: prefer Claude's own number, else derive from transcript usage.
  const ctxPct =
    sl?.contextUsedPct ??
    (usage && usage.contextLimit > 0 ? (usage.contextTokens / usage.contextLimit) * 100 : undefined);

  // Cumulative tokens: statusLine carries in+out; fall back to usage.
  const tokens =
    sl?.totalInputTokens !== undefined || sl?.totalOutputTokens !== undefined
      ? (sl.totalInputTokens ?? 0) + (sl.totalOutputTokens ?? 0)
      : usage
        ? usage.totalInputTokens + usage.totalOutputTokens
        : undefined;

  return {
    model,
    ctxPct,
    tokens,
    costUSD: sl?.costUSD ?? usage?.costUSD,
    fiveHourPct: sl?.fiveHourPct,
    sevenDayPct: sl?.sevenDayPct,
  };
}
