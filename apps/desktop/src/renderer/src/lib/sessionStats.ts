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
