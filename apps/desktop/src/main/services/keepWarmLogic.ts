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
