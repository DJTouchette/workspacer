// Pure decision logic for the keep-warm 5h-window pinger — separated from
// keepWarmService so tests can exercise it without pulling in the config/
// daemon singletons. See keepWarmService.ts for the feature overview.
//
// Two gates compose:
//  - the SCHEDULE gate (per config mode: always / every N hours / daily at) —
//    one shared slot regardless of how many providers are warmed;
//  - the PROVIDER gate (per provider: failure backoff + a window we already
//    know/assume is running) — Claude and Codex windows lapse independently.

export interface KeepWarmConfig {
  enabled: boolean;
  /** Which subscription windows to warm: 'claude' and/or 'codex'. */
  providers: string[];
  mode: 'auto' | 'interval' | 'daily';
  intervalHours: number;
  dailyAt: string;
}

/** Shared scheduling state — when the last interval/daily slot ran. */
export interface ScheduleState {
  lastIntervalCheckMs: number | null;
  lastDailyKey: string | null;
}

/** Per-provider window state. */
export interface ProviderWarmState {
  /** Reset time (epoch ms) of the window we know/assume is running. */
  assumedResetsAtMs: number | null;
  /** Failure backoff: no ping attempts before this (epoch ms). */
  notBeforeMs: number | null;
}

export const emptySchedule = (): ScheduleState => ({
  lastIntervalCheckMs: null,
  lastDailyKey: null,
});

export const emptyProviderState = (): ProviderWarmState => ({
  assumedResetsAtMs: null,
  notBeforeMs: null,
});

/** Local calendar key (YYYY-MM-DD) for daily-mode dedup. */
export function dayKey(now: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

/** Whether this tick opens a check slot at all, per mode. Pure. */
export function scheduleDue(cfg: KeepWarmConfig, state: ScheduleState, now: Date): boolean {
  switch (cfg.mode) {
    case 'auto':
      return true;
    case 'interval': {
      const hours = cfg.intervalHours > 0 ? cfg.intervalHours : 5;
      return (
        state.lastIntervalCheckMs == null ||
        now.getTime() - state.lastIntervalCheckMs >= hours * 60 * 60 * 1000
      );
    }
    case 'daily': {
      if (state.lastDailyKey === dayKey(now)) return false;
      const m = /^(\d{1,2}):(\d{2})$/.exec(cfg.dailyAt.trim());
      if (!m) return false; // unparseable time — do nothing rather than guess
      const target = Number(m[1]) * 60 + Number(m[2]);
      return now.getHours() * 60 + now.getMinutes() >= target;
    }
    default:
      return false;
  }
}

/** Whether a provider needs its window checked (not backing off, and no
 *  window known/assumed to be running). Pure. */
export function providerNeedsCheck(state: ProviderWarmState, nowMs: number): boolean {
  if (state.notBeforeMs != null && nowMs < state.notBeforeMs) return false;
  if (state.assumedResetsAtMs != null && nowMs < state.assumedResetsAtMs) return false;
  return true;
}

/** Account usage as served by claudemon's GET /usage (snake_case wire). */
export interface AccountUsageWire {
  five_hour_pct?: number | null;
  five_hour_resets_at?: number | null; // epoch seconds
}

/** Whether a 5h window is currently running. The OAuth endpoint has been seen
 *  reporting utilization without `resets_at`, so either signal counts. */
export function windowActive(usage: AccountUsageWire, nowMs: number): boolean {
  if (usage.five_hour_pct != null && usage.five_hour_pct > 0) return true;
  if (usage.five_hour_resets_at != null && usage.five_hour_resets_at * 1000 > nowMs) return true;
  return false;
}

// ---- claudemon's GET /usage/report ------------------------------------
//
// The sessionless half of the window question. `/usage` answers only for the
// DEFAULT Claude login and will make a blocking network fetch to do it;
// `/usage/report` answers for every provider and every configured account from
// what each CLI already left on disk, immediately and with nothing running.
// Only the 5h window is typed here — the document is far wider (spend, tokens,
// per-model splits) and keep-warm reads none of it.

/** One scalar in the report. `ok` carries a reading; the other two carry a
 *  reason instead, and are NOT zero. */
export interface UsageReportMeasured {
  state: 'ok' | 'unknown' | 'unavailable';
  value?: number | null;
  reason?: string | null;
}

export interface UsageReportWindow {
  used_percent?: UsageReportMeasured | null;
  /** Epoch seconds. `null` when the source reported no reset time, which is
   *  also why the report leaves `is_current` null there. */
  resets_at?: number | null;
  window_minutes?: number | null;
  is_current?: boolean | null;
}

export interface UsageReportAccount {
  /** `""` is the default login; `null` means the daemon cannot say which. */
  account?: string | null;
  label?: string | null;
  windows?: { five_hour?: UsageReportWindow | null } | null;
}

export interface UsageReportProvider {
  provider: string;
  accounts?: UsageReportAccount[] | null;
}

export interface UsageReportWire {
  generated_at?: number;
  providers?: UsageReportProvider[] | null;
}

/**
 * A provider's 5h window as the report sees it, in the flat shape
 * `windowActive` consumes. Three outcomes, and the difference between the last
 * two is the whole value of reading the report at all:
 *
 *   `{five_hour_resets_at}` — a window is running, and this is when it lapses.
 *   `{}`                    — every readable reading has ALREADY rolled over.
 *                             A definite "no window is running", so the caller
 *                             can ping without claiming it was in the dark.
 *   `null`                  — nothing readable. Genuinely unknown; ask a
 *                             different source.
 *
 * The PERCENTAGE is deliberately not carried across. `used_percent` is the last
 * figure the provider wrote, and for a rolled-over window that is real history
 * and a false present — Codex reads 67% on a `resets_at` two days gone.
 * `windowActive` treats any percentage above zero as a live window, so
 * forwarding it would suppress warming permanently. Currency is decided from
 * `resets_at` against `nowMs` rather than from the report's own `is_current`,
 * which was computed at `generated_at` and is the staler of the two.
 */
export function fiveHourWindowFromReport(
  report: UsageReportWire,
  provider: string,
  nowMs: number,
): AccountUsageWire | null {
  const p = (report.providers ?? []).find((x) => x?.provider === provider);
  if (!p) return null;
  let best: number | null = null;
  let lapsed = false;
  for (const acct of p.accounts ?? []) {
    const resets = acct?.windows?.five_hour?.resets_at;
    // No reset time is no answer: a percentage alone cannot say which window
    // it describes, which is exactly why the report leaves is_current null.
    if (resets == null) continue;
    if (resets * 1000 > nowMs) {
      if (best == null || resets > best) best = resets;
    } else {
      lapsed = true;
    }
  }
  if (best != null) return { five_hour_resets_at: best };
  return lapsed ? {} : null;
}
