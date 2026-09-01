import { DRIFT_TOLERANCE } from '../../../main/shared/modelContextWindows';
import type {
  ClaudeSessionSnapshot,
  FileChange,
  SessionPlan,
  SessionStatusLine,
  SessionUsage,
} from '../types/claudeSession';
import { collectEditedFiles } from './turnChanges';

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
  // Switch to "M" once rounding the thousands would reach 1000 — otherwise
  // values in [999_500, 999_999] round to a 4-digit "1000k".
  if (n >= 1_000_000 || Math.round(n / 1_000) >= 1_000)
    return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return `${n}`;
}

/** Cost, compact: `$1.23`, `<$0.01` for tiny non-zero, `$0.00` for nothing. */
export function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}

/**
 * Compact time-until-reset for a unix-seconds timestamp: "38m", "5h", "3d"
 * (single coarsest unit — this rides in tight status readouts). Undefined when
 * absent or already past, so callers can just skip rendering.
 */
export function fmtResetIn(epochSecs?: number): string | undefined {
  if (!epochSecs) return undefined;
  const mins = Math.round((epochSecs * 1000 - Date.now()) / 60000);
  if (mins <= 0) return undefined;
  if (mins < 60) return `${mins}m`;
  const h = Math.round(mins / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

/** Absolute local wall-clock for a unix-seconds timestamp — tooltip detail. */
export function fmtResetAt(epochSecs?: number): string | undefined {
  if (!epochSecs) return undefined;
  return new Date(epochSecs * 1000).toLocaleString([], {
    weekday: 'short',
    hour: 'numeric',
    minute: '2-digit',
  });
}

/** Context-window fill color by PERCENT (0–100): green → amber (≥70) → red (≥90). */
export function ctxColor(pct: number): string {
  if (pct >= 90) return 'var(--wks-error)';
  if (pct >= 70) return 'var(--wks-warning)';
  return 'var(--wks-success)';
}

/** How long a "working" agent may go without a snapshot update before we flag it. */
export const STALE_AFTER_MS = 120_000;

/**
 * A snapshot is stale when the agent CLAIMS to be working but nothing has
 * arrived for {@link STALE_AFTER_MS} — usually a died stream or wedged session,
 * exactly the case where the card's confident "Working" label misleads.
 * Idle/waiting agents are never stale (silence is their normal state).
 */
export function isSnapshotStale(
  ambientState: string | undefined,
  lastActivity: number | undefined,
  now: number,
): boolean {
  if (ambientState !== 'thinking' && ambientState !== 'streaming') return false;
  return !!lastActivity && now - lastActivity > STALE_AFTER_MS;
}

/**
 * One-line change summary for a session's whole `fileChanges` list — the
 * "~ 4 files · +120 −36" line on fleet cards. Line counts are tool-input
 * estimates (same math as the per-turn ChangedFilesCard fallback), so both
 * surfaces agree.
 */
export function summarizeFileChanges(fileChanges: FileChange[]): {
  files: number;
  added: number;
  removed: number;
} {
  const edited = collectEditedFiles(
    fileChanges.map((fc) => ({
      id: '',
      // fc.path is authoritative even when the tool input spells it another way
      name: fc.toolName,
      input: { ...fc.input, file_path: fc.input?.file_path ?? fc.path },
      status: 'complete' as const,
      startedAt: fc.timestamp,
    })),
  );
  let added = 0;
  let removed = 0;
  for (const est of edited.values()) {
    added += est.added;
    removed += est.removed;
  }
  return { files: edited.size, added, removed };
}

export interface PlanProgress {
  done: number;
  total: number;
  /** The step currently in_progress, if any — its activeForm is the live line. */
  active: SessionPlan['steps'][number] | undefined;
}

/**
 * Collapse a plan into the counts every plan surface needs: completed/total and
 * the in_progress step (whose activeForm is the "doing now" line). Returns null
 * when there's no plan to show, so callers can just skip rendering.
 */
export function planProgress(plan: SessionPlan | undefined): PlanProgress | null {
  const steps = plan?.steps;
  if (!steps || steps.length === 0) return null;
  const done = steps.filter((s) => s.status === 'completed').length;
  const active = steps.find((s) => s.status === 'in_progress');
  return { done, total: steps.length, active };
}

export interface DerivedSessionStats {
  model?: string;
  /** Context window fill, 0–100. */
  ctxPct?: number;
  /** CURRENT WINDOW OCCUPANCY — what this session is holding right now, the
   *  numerator of `ctxPct`. This is what a glanceable agent surface means by
   *  "how big is this agent", and it is bounded by the window. */
  contextTokens?: number;
  /** Provider-confirmed effective denominator. Never a requested/configured estimate. */
  effectiveContextWindow?: number;
  /** CUMULATIVE PROMPT+COMPLETION TOKENS BILLED over the whole session — the
   *  COST-SIDE figure, not an occupancy. Every API call re-sends the entire
   *  conversation, and on Claude nearly all of it comes back as
   *  `cache_read_input_tokens`, so this counts the same ~200k of context once
   *  per turn: a 141-call worker holding 356k tokens has legitimately billed
   *  30M. That is arithmetic, not a bug (the accumulators dedupe by message id
   *  and were verified against live frames to the token) — but rendered beside
   *  a context bar as a bare "30M tok" it reads as occupancy, which is the
   *  "worker shows 23M tokens" report. It belongs only on surfaces that say
   *  what it is. Named `billedTokens`, not `tokens`, so no caller can inherit
   *  the ambiguity by accident. */
  billedTokens?: number;
  costUSD?: number;
  fiveHourPct?: number;
  /** Unix epoch seconds the 5h rate-limit window resets at. */
  fiveHourResetsAt?: number;
  /** The 5h window's length in minutes (300 for Claude; Codex reports its own,
   *  which is why this is carried rather than assumed). */
  fiveHourWindowMins?: number;
  sevenDayPct?: number;
  /** Unix epoch seconds the 7d rate-limit window resets at. */
  sevenDayResetsAt?: number;
  /** The weekly window's length in minutes (10080 for Claude). */
  sevenDayWindowMins?: number;
  /** Monthly overage/credit window utilization (stream `overage` type). */
  monthlyPct?: number;
  /** Unix epoch seconds the monthly overage window resets at. */
  monthlyResetsAt?: number;
  /** The monthly window's length in minutes. No source reports one today. */
  monthlyWindowMins?: number;
  /** Active "approaching a limit" warning (stream only). */
  rateLimitWarning?: string;
  /** Monthly overage disabled for lack of credits (stream only). */
  overageOutOfCredits?: boolean;
  /** At least one of `costUSD` / `billedTokens` came from the history DB's last
   *  recorded snapshot rather than a live one — see {@link withRecordedUsage}.
   *  Surfaces use it to say "last recorded" instead of implying live. */
  recorded?: boolean;
}

/**
 * What {@link deriveSessionStats} actually needs. Kept looser than a whole
 * snapshot because account-scoped surfaces (the Overview's usage card) hold a
 * statusLine and no session at all, and they must be able to derive the same
 * numbers as a pane that does.
 */
export type SessionStatsSource = Partial<Pick<ClaudeSessionSnapshot, 'usage' | 'statusLine'>>;

export function deriveSessionStats(snapshot?: SessionStatsSource | null): DerivedSessionStats {
  const sl: SessionStatusLine | undefined = snapshot?.statusLine;
  const usage: SessionUsage | null | undefined = snapshot?.usage;

  const model =
    sl?.modelDisplay ?? (usage?.model ? usage.model.replace(/^claude-/, '') : undefined);

  // A context BAR is evidence about a live runtime, not about configuration.
  // Requested/persisted/catalog windows are useful labels in the Context
  // popover, but may not be denominators here. Only the provider-reported
  // status-line pair confirms the effective window.
  const slWindow =
    sl?.contextWindowSize && Number.isFinite(sl.contextWindowSize) && sl.contextWindowSize > 0
      ? sl.contextWindowSize
      : undefined;
  const held = usage?.contextTokens ?? 0;
  const slWindowDisproved = !!slWindow && held > slWindow * DRIFT_TOLERANCE;
  const ctxPct =
    slWindow && !slWindowDisproved
      ? sl?.contextUsedPct !== undefined
        ? sl.contextUsedPct
        : held > 0
          ? (held / slWindow) * 100
          : undefined
      : undefined;

  // Occupancy, in tokens. The transcript counts it directly; a managed provider
  // that only reports a percentage has it reconstructed from the pair — but
  // never from a window the line above just disproved, since pct × a wrong
  // window is how an absurd token count gets manufactured downstream.
  const contextTokens =
    !slWindowDisproved && sl?.contextUsedPct !== undefined && slWindow
      ? Math.round((sl.contextUsedPct / 100) * slWindow)
      : usage && usage.contextTokens > 0
        ? usage.contextTokens
        : undefined;

  // Cumulative BILLED tokens: statusLine carries in+out; fall back to usage.
  const billedTokens =
    sl?.totalInputTokens !== undefined || sl?.totalOutputTokens !== undefined
      ? (sl.totalInputTokens ?? 0) + (sl.totalOutputTokens ?? 0)
      : usage
        ? usage.totalInputTokens + usage.totalOutputTokens
        : undefined;

  return {
    model,
    ctxPct,
    contextTokens,
    effectiveContextWindow: slWindowDisproved ? undefined : slWindow,
    billedTokens,
    costUSD: sl?.costUSD ?? usage?.costUSD,
    fiveHourPct: sl?.fiveHourPct,
    fiveHourResetsAt: sl?.fiveHourResetsAt,
    fiveHourWindowMins: sl?.fiveHourWindowMins,
    sevenDayPct: sl?.sevenDayPct,
    sevenDayResetsAt: sl?.sevenDayResetsAt,
    sevenDayWindowMins: sl?.sevenDayWindowMins,
    monthlyPct: sl?.monthlyPct,
    monthlyResetsAt: sl?.monthlyResetsAt,
    monthlyWindowMins: sl?.monthlyWindowMins,
    rateLimitWarning: sl?.rateLimitWarning,
    overageOutOfCredits: sl?.overageOutOfCredits,
  };
}

// ── Recorded (last-known) usage ──────────────────────────────────────────────
//
// Everything above derives from a LIVE session snapshot. At a cold start there
// are no live snapshots: claudemon reports a restored agent's session as a
// stopped/ended row, `promoteSessionSnapshots` deliberately drops those, and
// every cost/token slot on every agent surface therefore renders a dash — even
// though the desktop's own history DB has the figures and the recent-sessions
// join already carries them across IPC.
//
// This is the seam that puts them back: a surface merges the recorded figures
// UNDER the live ones, so a live reading always wins and a cold one is filled
// from what was last recorded. Absence stays absence — nothing here invents a
// zero, and a session with no recorded usage merges to exactly what it was.

/** What the history DB recorded for a session, as it crosses IPC. Each field is
 *  undefined when nothing was recorded — never 0 (see RecentAgentSession). */
export interface RecordedSessionUsage {
  costUSD?: number;
  billedTokens?: number;
}

/**
 * Fill a session's missing cost/token figures from what was last recorded.
 *
 * Only fields the live snapshot left UNDEFINED are filled: a live $0.03 is
 * never overwritten by a stale record, and a recorded absence never erases a
 * live reading. `recorded` is set when at least one figure came from the record
 * rather than from a live snapshot, so a surface can label it as last-known
 * instead of implying it is current.
 *
 * Returns the SAME object when there is nothing to fill — that identity is
 * load-bearing for SideBar's per-snapshot stats cache.
 */
export function withRecordedUsage(
  stats: DerivedSessionStats,
  recorded: RecordedSessionUsage | undefined,
): DerivedSessionStats {
  if (!recorded) return stats;
  const cost = stats.costUSD === undefined ? recorded.costUSD : undefined;
  const tokens = stats.billedTokens === undefined ? recorded.billedTokens : undefined;
  if (cost === undefined && tokens === undefined) return stats;
  return {
    ...stats,
    ...(cost !== undefined && { costUSD: cost }),
    ...(tokens !== undefined && { billedTokens: tokens }),
    recorded: true,
  };
}

// ── Prompt cache ─────────────────────────────────────────────────────────────
//
// Every request re-reads the whole conversation before the model answers, and
// the providers report how that prompt divided: processed fresh, written into
// the cache, or served back from it. Workspacer priced those three tiers and
// then summed them away, so the split existed only inside the cost arithmetic.
//
// Same rule as `usageWindows` below: a figure appears only when a provider
// actually reported it. There is no zero-filling here, because "this provider
// does not itemize cache writes" and "this session wrote nothing to cache" are
// different claims and only one of them is ours to make.

/** A session's prompt-cache split, ready to render. */
export interface CacheBreakdown {
  /** Prompt tokens processed fresh, at the full input rate. */
  fresh: number;
  /** Prompt tokens written into the cache. Undefined when the provider counts
   *  cache reads but never itemizes writes (Codex reports a cached subset of
   *  its input and nothing about writes). A 0 would claim it wrote nothing. */
  write?: number;
  /** Prompt tokens served back from the cache. */
  read: number;
  /** Prompt tokens across every tier the provider reported. */
  total: number;
  /** Share of the prompt served from cache, 0-100. Undefined when `total` is 0:
   *  a hit rate over an empty prompt is undefined, not 0%, and 0% would read as
   *  a cache that never hit. */
  hitRatePct?: number;
}

/**
 * The prompt-cache split for a session, from whichever source actually has it.
 *
 * Claude's itemized transcript split wins, because it is the only source that
 * separates writes from reads. Codex has no such itemization, but its status
 * line does carry a cache-read subset of the cumulative input, which is enough
 * for the fresh/read halves. Null when neither source reported anything, so
 * callers omit the section rather than draw an all-zero one.
 *
 * TWIN: `cache_report` in apps/tui/src/types.rs.
 */
export function cacheBreakdown(snapshot?: SessionStatsSource | null): CacheBreakdown | null {
  const withTotals = (fresh: number, write: number | undefined, read: number): CacheBreakdown => {
    const total = fresh + (write ?? 0) + read;
    return {
      fresh,
      write,
      read,
      total,
      hitRatePct: total > 0 ? (read / total) * 100 : undefined,
    };
  };
  const c = snapshot?.usage?.cache;
  if (c) return withTotals(c.fresh, c.write, c.read);
  const sl = snapshot?.statusLine;
  if (sl?.cachedInputTokens === undefined || sl.totalInputTokens === undefined) return null;
  const read = Math.min(sl.cachedInputTokens, sl.totalInputTokens);
  return withTotals(sl.totalInputTokens - read, undefined, read);
}

// ── Account rate-limit windows ───────────────────────────────────────────────
//
// Every surface that draws windows (the status bar chip, its detail modal, the
// Inspector card, the Overview card) builds its list from `usageWindows` rather
// than hard-coding three rows. The reason is that the set is genuinely variable:
// Claude has no monthly window unless extra usage is enabled on the account, and
// Codex reports two windows and never a monthly one. A hard-coded list draws a
// dead 0% meter for whatever is missing; this list simply omits it, and picks
// the monthly window back up on its own if the account ever enables overage.

/** One rolling account window, as the provider actually reported it. */
export interface UsageWindow {
  key: 'fiveHour' | 'sevenDay' | 'monthly';
  /** Cramped-surface label: the window's own length when known (`5h`, `7d`),
   *  else the slot's name. */
  short: string;
  /** Full label for roomy surfaces: `5-hour limit`. */
  label: string;
  /** Utilization 0–100. Undefined when only a reset time arrived. */
  pct?: number;
  /** Unix epoch seconds this window resets at. */
  resetsAt?: number;
  /** Window length in minutes, when the provider reports or implies one. */
  windowMins?: number;
}

/** Compact window length: `5h`, `7d`, `90m`. Undefined when unknown. */
export function fmtWindowShort(mins?: number): string | undefined {
  if (!mins || mins <= 0) return undefined;
  if (mins % 1440 === 0) return `${mins / 1440}d`;
  if (mins % 60 === 0) return `${mins / 60}h`;
  return `${mins}m`;
}

/** Spelled-out window length: `5 hours`, `7 days`, `1 hour 30 min`. */
export function fmtWindowLength(mins?: number): string | undefined {
  if (!mins || mins <= 0) return undefined;
  const unit = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;
  if (mins % 1440 === 0) return unit(mins / 1440, 'day');
  if (mins % 60 === 0) return unit(mins / 60, 'hour');
  if (mins > 60) return `${unit(Math.floor(mins / 60), 'hour')} ${mins % 60} min`;
  return `${mins} min`;
}

/** Attributive form for a window label: `5-hour`, `7-day`, `90-minute`. Feeds
 *  "5-hour limit", so the label states the real duration rather than assuming
 *  the slot's conventional one (a Codex primary window need not be 5 hours). */
export function fmtWindowAdjective(mins?: number): string | undefined {
  if (!mins || mins <= 0) return undefined;
  if (mins % 1440 === 0) return `${mins / 1440}-day`;
  if (mins % 60 === 0) return `${mins / 60}-hour`;
  return `${mins}-minute`;
}

/**
 * The account windows this session actually has data for, in order.
 *
 * A window earns a row when it carries a percentage OR a reset time; a slot with
 * neither is not "0% used", it is not reported, and rendering it would invent a
 * meter. `windowMins` rides along so the label can say how long the window is
 * instead of leaving it to the slot's name.
 */
/** The window fields `usageWindows` needs. Both `DerivedSessionStats` and the
 *  raw `SessionStatusLine` satisfy it, so either can be passed. */
export type UsageWindowSource = Partial<
  Pick<
    DerivedSessionStats,
    | 'fiveHourPct'
    | 'fiveHourResetsAt'
    | 'fiveHourWindowMins'
    | 'sevenDayPct'
    | 'sevenDayResetsAt'
    | 'sevenDayWindowMins'
    | 'monthlyPct'
    | 'monthlyResetsAt'
    | 'monthlyWindowMins'
  >
>;

export function usageWindows(stats: UsageWindowSource): UsageWindow[] {
  const slots: Array<
    Pick<UsageWindow, 'key' | 'label' | 'pct' | 'resetsAt' | 'windowMins'> & { fallback: string }
  > = [
    {
      key: 'fiveHour',
      label: '5-hour limit',
      fallback: '5h',
      pct: stats.fiveHourPct,
      resetsAt: stats.fiveHourResetsAt,
      windowMins: stats.fiveHourWindowMins,
    },
    {
      key: 'sevenDay',
      label: '7-day limit',
      fallback: '7d',
      pct: stats.sevenDayPct,
      resetsAt: stats.sevenDayResetsAt,
      windowMins: stats.sevenDayWindowMins,
    },
    {
      key: 'monthly',
      label: 'Monthly limit',
      fallback: 'Mo',
      pct: stats.monthlyPct,
      resetsAt: stats.monthlyResetsAt,
      windowMins: stats.monthlyWindowMins,
    },
  ];
  return slots
    .filter((s) => s.pct !== undefined || s.resetsAt !== undefined)
    .map(({ fallback, label, ...s }) => {
      const adj = fmtWindowAdjective(s.windowMins);
      return {
        ...s,
        label: adj ? `${adj} limit` : label,
        short: fmtWindowShort(s.windowMins) ?? fallback,
      };
    });
}
