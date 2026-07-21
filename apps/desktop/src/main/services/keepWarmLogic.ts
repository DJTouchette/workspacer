// Pure decision logic for the keep-warm 5h-window pinger — separated from
// keepWarmService so tests can exercise it without pulling in the config/
// daemon singletons. See keepWarmService.ts for the feature overview.

export interface KeepWarmConfig {
  enabled: boolean;
  mode: 'auto' | 'interval' | 'daily';
  intervalHours: number;
  dailyAt: string;
}

export interface KeepWarmState {
  /** Reset time (epoch ms) of the window we know/assume is running. */
  assumedResetsAtMs: number | null;
  /** Last time the interval-mode check ran (epoch ms). */
  lastIntervalCheckMs: number | null;
  /** Local YYYY-MM-DD of the last daily-mode check. */
  lastDailyKey: string | null;
  /** Failure backoff: no ping attempts before this (epoch ms). */
  notBeforeMs: number | null;
}

export const emptyKeepWarmState = (): KeepWarmState => ({
  assumedResetsAtMs: null,
  lastIntervalCheckMs: null,
  lastDailyKey: null,
  notBeforeMs: null,
});

/** Local calendar key (YYYY-MM-DD) for daily-mode dedup. */
export function dayKey(now: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

/** Whether this tick should run the usage check at all, per mode. */
export function dueForCheck(cfg: KeepWarmConfig, state: KeepWarmState, now: Date): boolean {
  const nowMs = now.getTime();
  if (state.notBeforeMs != null && nowMs < state.notBeforeMs) return false;
  // A window we know/assume is running makes every mode quiet — the whole
  // feature is "start one when there isn't one".
  if (state.assumedResetsAtMs != null && nowMs < state.assumedResetsAtMs) return false;
  switch (cfg.mode) {
    case 'auto':
      return true;
    case 'interval': {
      const hours = cfg.intervalHours > 0 ? cfg.intervalHours : 5;
      return (
        state.lastIntervalCheckMs == null ||
        nowMs - state.lastIntervalCheckMs >= hours * 60 * 60 * 1000
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
