// Keep-warm — keeps a subscription 5-hour rate-limit window running.
//
// Claude subscription usage is metered in 5-hour windows that only START when
// the first message of a window is sent. Sitting down at 9:00 and sending your
// first real prompt at 9:30 "wastes" half an hour of window. When enabled,
// this service checks the account's current 5h window (via claudemon's
// ungated GET /usage) and, when it is expired/absent (0%), fires ONE minimal
// headless Haiku ping (`claude -p … --model haiku`) so a fresh window is
// already running. It never pings while a window is active.
//
// Modes ('claude.keepWarm.mode'):
//   auto     — re-warm whenever the window lapses (window always running)
//   interval — check every `intervalHours`
//   daily    — check once a day at `dailyAt` (local "HH:MM")
//
// The ping itself is a claudemon "heartbeat" (POST /heartbeat): the daemon
// runs the turn through the same stream-json contract as managed sessions —
// one owner of the CLI wire contract, not a parallel `-p` path here — records
// it in its heartbeats table (never a session, so warms can't surface in the
// sidebar), and returns the new window's reset time. We remember that reset
// (or assume now+5h) so `auto` mode stays quiet — no network, no re-ping —
// until the window actually lapses. Off by default; only runs while
// Workspacer is open. Failure is always soft and log-only.
//
// Decision logic lives in keepWarmLogic.ts (pure, unit-tested).
import { configService } from './configService';
import { claudeBaseArgv } from './claudeResolver';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import {
  AccountUsageWire,
  KeepWarmConfig,
  KeepWarmState,
  dayKey,
  dueForCheck,
  emptyKeepWarmState,
  windowActive,
} from './keepWarmLogic';

const CHECK_INTERVAL_MS = 60_000;
const FIVE_HOURS_MS = 5 * 60 * 60 * 1000;
/** The daemon caps a heartbeat run at 120s; allow a little slack on top. */
const PING_TIMEOUT_MS = 130_000;
const USAGE_TIMEOUT_MS = 15_000;
/** After a failed ping, don't retry for this long (auto mode would otherwise
 *  re-attempt every tick while e.g. offline). */
const FAILURE_BACKOFF_MS = 10 * 60 * 1000;

class KeepWarmService {
  private timer: ReturnType<typeof setInterval> | null = null;
  private inFlight = false;
  private state: KeepWarmState = emptyKeepWarmState();

  start(): void {
    if (this.timer) return;
    void this.tick();
    this.timer = setInterval(() => void this.tick(), CHECK_INTERVAL_MS);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  private readConfig(): KeepWarmConfig | null {
    const kw = configService.getConfig().claude?.keepWarm;
    if (!kw?.enabled) return null;
    return {
      enabled: true,
      mode: kw.mode ?? 'auto',
      intervalHours: kw.intervalHours ?? 5,
      dailyAt: kw.dailyAt ?? '08:00',
    };
  }

  private async tick(): Promise<void> {
    if (this.inFlight) return;
    const cfg = this.readConfig();
    if (!cfg) return;
    const now = new Date();
    if (!dueForCheck(cfg, this.state, now)) return;
    this.inFlight = true;
    try {
      // Mark the check for interval/daily dedup no matter how it turns out —
      // "checked and window was warm" counts as this slot's check.
      if (cfg.mode === 'interval') this.state.lastIntervalCheckMs = now.getTime();
      if (cfg.mode === 'daily') this.state.lastDailyKey = dayKey(now);

      const usage = await this.fetchUsage();
      if (usage && windowActive(usage, now.getTime())) {
        if (usage.five_hour_resets_at != null) {
          this.state.assumedResetsAtMs = usage.five_hour_resets_at * 1000;
        }
        console.log('[keepWarm] 5h window already active — no ping needed');
        return;
      }
      if (!usage) {
        console.log('[keepWarm] account usage unavailable — pinging to be safe');
      }
      await this.ping(now);
    } finally {
      this.inFlight = false;
    }
  }

  /** claudemon's ungated account-usage endpoint; null = unknown. */
  private async fetchUsage(): Promise<AccountUsageWire | null> {
    try {
      const res = await fetch(`${CLAUDEMON_API_URL}/usage`, {
        signal: AbortSignal.timeout(USAGE_TIMEOUT_MS),
      });
      if (!res.ok) return null;
      return (await res.json()) as AccountUsageWire;
    } catch {
      return null;
    }
  }

  /** One claudemon heartbeat — a minimal Haiku turn run by the daemon. */
  private async ping(now: Date): Promise<void> {
    console.log('[keepWarm] requesting heartbeat from claudemon');
    let ok = false;
    let resetsAtSec: number | null = null;
    try {
      const res = await fetch(`${CLAUDEMON_API_URL}/heartbeat`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ argv: claudeBaseArgv(), model: 'haiku' }),
        signal: AbortSignal.timeout(PING_TIMEOUT_MS),
      });
      if (res.ok) {
        const row = (await res.json()) as {
          ok: boolean;
          resets_at?: number | null;
          error?: string | null;
        };
        ok = row.ok;
        resetsAtSec = row.resets_at ?? null;
        if (!row.ok) console.log(`[keepWarm] heartbeat failed: ${row.error ?? 'unknown'}`);
      } else {
        console.log(`[keepWarm] heartbeat endpoint returned ${res.status}`);
      }
    } catch (err) {
      console.log(`[keepWarm] heartbeat request failed: ${(err as Error).message}`);
    }
    if (!ok) {
      this.state.notBeforeMs = now.getTime() + FAILURE_BACKOFF_MS;
      return;
    }
    this.state.notBeforeMs = null;
    const resetsAtMs = resetsAtSec != null ? resetsAtSec * 1000 : now.getTime() + FIVE_HOURS_MS;
    this.state.assumedResetsAtMs = resetsAtMs;
    console.log(
      `[keepWarm] window started — resets at ${new Date(resetsAtMs).toLocaleTimeString()}`,
    );
  }
}

export const keepWarmService = new KeepWarmService();
