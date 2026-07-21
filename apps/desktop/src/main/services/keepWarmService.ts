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
// The ping self-reports the new window: headless stream-json output includes a
// `rate_limit_event` whose `rate_limit_info.resetsAt` is the new 5h reset. We
// remember it (or assume now+5h) so `auto` mode stays quiet — no network, no
// re-ping — until the window actually lapses. Off by default; only runs while
// Workspacer is open. Failure is always soft and log-only.
//
// Decision logic lives in keepWarmLogic.ts (pure, unit-tested).
import { spawn } from 'child_process';
import os from 'os';
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
  parsePingResetsAtMs,
  windowActive,
} from './keepWarmLogic';

const CHECK_INTERVAL_MS = 60_000;
const FIVE_HOURS_MS = 5 * 60 * 60 * 1000;
const PING_TIMEOUT_MS = 120_000;
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

  /** One minimal Haiku turn — the cheapest thing that starts a window. */
  private async ping(now: Date): Promise<void> {
    const [bin, ...base] = claudeBaseArgv();
    const argv = [
      ...base,
      '-p',
      'Reply with exactly: ok',
      '--model',
      'haiku',
      '--output-format',
      'stream-json',
      '--verbose',
    ];
    console.log(`[keepWarm] pinging (${bin} ${argv.join(' ')})`);
    const result = await new Promise<{ ok: boolean; stdout: string }>((resolve) => {
      const child = spawn(bin, argv, {
        cwd: os.homedir(),
        env: process.env,
        stdio: ['ignore', 'pipe', 'pipe'],
        timeout: PING_TIMEOUT_MS,
      });
      let stdout = '';
      let stderr = '';
      child.stdout.on('data', (d) => (stdout += String(d)));
      child.stderr.on('data', (d) => (stderr += String(d)));
      child.on('error', (err) => {
        console.log(`[keepWarm] ping failed to spawn: ${err.message}`);
        resolve({ ok: false, stdout });
      });
      child.on('close', (code) => {
        if (code !== 0) {
          console.log(`[keepWarm] ping exited ${code}: ${stderr.trim().slice(0, 400)}`);
        }
        resolve({ ok: code === 0, stdout });
      });
    });
    if (!result.ok) {
      this.state.notBeforeMs = now.getTime() + FAILURE_BACKOFF_MS;
      return;
    }
    this.state.notBeforeMs = null;
    const resetsAtMs = parsePingResetsAtMs(result.stdout) ?? now.getTime() + FIVE_HOURS_MS;
    this.state.assumedResetsAtMs = resetsAtMs;
    console.log(
      `[keepWarm] window started — resets at ${new Date(resetsAtMs).toLocaleTimeString()}`,
    );
  }
}

export const keepWarmService = new KeepWarmService();
