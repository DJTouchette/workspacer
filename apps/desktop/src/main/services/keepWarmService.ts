// Keep-warm — keeps subscription 5-hour rate-limit windows running.
//
// Claude and Codex subscriptions both meter usage in 5-hour windows that only
// START when the first message of a window is sent. Sitting down at 9:00 and
// sending your first real prompt at 9:30 "wastes" half an hour of window.
// When enabled, this service checks each configured provider's current 5h
// window and, when it is expired/absent (0%), fires ONE minimal turn so a
// fresh window is already running. It never pings while a window is active.
//
// Modes ('claude.keepWarm.mode'):
//   auto     — re-warm whenever a window lapses (windows always running)
//   interval — check every `intervalHours`
//   daily    — check once a day at `dailyAt` (local "HH:MM")
//
// The ping itself is a claudemon "heartbeat" (POST /heartbeat): the daemon
// runs the turn through the same wire contract as managed sessions — the
// stream adapter for Claude, a throwaway app-server for Codex; one owner per
// CLI contract — records it in its heartbeats table (never a session, so
// warms can't surface in the sidebar), and returns the new window's reset
// time. We remember that reset (or assume now+5h) so `auto` mode stays quiet
// until the window actually lapses.
//
// Window checks before pinging — both sessionless, neither needs the provider
// to be running:
//   claude — claudemon's ungated GET /usage. It answers for the DEFAULT login,
//            which is the one `argvFor` warms, and it refetches on demand when
//            its cached reading has gone stale.
//   codex  — claudemon's GET /usage/report, which reads the newest rollout the
//            Codex CLI left on disk. A live session's status line is the
//            fallback for the one case the rollout cannot answer (a reading
//            with no reset time, so nothing can say which window it describes).
// When both are silent, ping to be safe — a redundant ping costs a few tokens
// and cannot reset an already-running window.
//
// Off by default; only runs while Workspacer is open. Failure is always soft
// and log-only. Decision logic lives in keepWarmLogic.ts (pure, unit-tested).
import { configService } from './configService';
import { claudeBaseArgv } from './claudeResolver';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { resolveAgentBinary } from './agentProviders';
import { claudeSessionStore } from './claudeSessionStore';
import { isWarmableProvider } from '../shared/keepWarmProviders';
import {
  AccountUsageWire,
  KeepWarmConfig,
  ProviderWarmState,
  ScheduleState,
  UsageReportWire,
  dayKey,
  emptyProviderState,
  emptySchedule,
  fiveHourWindowFromReport,
  providerNeedsCheck,
  scheduleDue,
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
  private schedule: ScheduleState = emptySchedule();
  private providerState = new Map<string, ProviderWarmState>();
  private warnedNoWarmableProvider = false;

  start(): void {
    if (this.timer) return;
    void this.tick();
    this.timer = setInterval(() => void this.tick(), CHECK_INTERVAL_MS);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  private stateFor(provider: string): ProviderWarmState {
    let s = this.providerState.get(provider);
    if (!s) {
      s = emptyProviderState();
      this.providerState.set(provider, s);
    }
    return s;
  }

  private readConfig(): KeepWarmConfig | null {
    const kw = configService.getConfig().claude?.keepWarm;
    if (!kw?.enabled) return null;
    const configured = kw.providers ?? ['claude'];
    const providers = configured.filter(isWarmableProvider);
    if (!providers.length) {
      // Enabled, but nothing warmable is configured — the service would
      // otherwise do nothing at all, silently, beside a ticked checkbox. Not
      // reachable from Settings (its buttons emit only claude/codex), so this
      // is defensive: a hand-edited config.yaml or an imported one. Logged
      // once, because the tick is every 60s and forever.
      if (!this.warnedNoWarmableProvider) {
        this.warnedNoWarmableProvider = true;
        console.log(
          `[keepWarm] enabled, but none of [${configured.join(', ')}] has a 5h window to warm — doing nothing`,
        );
      }
      return null;
    }
    this.warnedNoWarmableProvider = false;
    return {
      enabled: true,
      providers,
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
    if (!scheduleDue(cfg, this.schedule, now)) return;
    const due = cfg.providers.filter((p) => providerNeedsCheck(this.stateFor(p), now.getTime()));
    // Interval/daily slots are consumed once opened — "checked, all warm"
    // counts as this slot's check. (Auto mode has no slot to consume.)
    if (cfg.mode === 'interval') this.schedule.lastIntervalCheckMs = now.getTime();
    if (cfg.mode === 'daily') this.schedule.lastDailyKey = dayKey(now);
    if (!due.length) return;
    this.inFlight = true;
    try {
      for (const provider of due) {
        await this.evaluate(provider, now);
      }
    } finally {
      this.inFlight = false;
    }
  }

  /** Check one provider's window; ping when it's expired/unknown. */
  private async evaluate(provider: string, now: Date): Promise<void> {
    const state = this.stateFor(provider);
    const known =
      provider === 'claude' ? await this.fetchClaudeUsage() : await this.codexWindow(now.getTime());
    if (known && windowActive(known, now.getTime())) {
      if (known.five_hour_resets_at != null) {
        state.assumedResetsAtMs = known.five_hour_resets_at * 1000;
      }
      console.log(`[keepWarm] ${provider} 5h window already active — no ping needed`);
      return;
    }
    if (!known) {
      console.log(`[keepWarm] ${provider} window state unknown — pinging to be safe`);
    } else {
      console.log(`[keepWarm] ${provider} 5h window has lapsed — warming`);
    }
    await this.ping(provider, now);
  }

  /** claudemon's ungated account-usage endpoint (Claude only); null = unknown. */
  private async fetchClaudeUsage(): Promise<AccountUsageWire | null> {
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

  /** claudemon's sessionless usage report — every provider's windows as their
   *  own CLIs left them on disk. null = unreadable.
   *
   *  THIS is /usage/report's first client. The route has served the widest read
   *  on the loopback plane since 0.160.0 with nothing in the repo constructing
   *  the URL; the capspec caller sweep is what found that out.
   */
  private async fetchUsageReport(): Promise<UsageReportWire | null> {
    try {
      const res = await fetch(`${CLAUDEMON_API_URL}/usage/report`, {
        signal: AbortSignal.timeout(USAGE_TIMEOUT_MS),
      });
      if (!res.ok) return null;
      return (await res.json()) as UsageReportWire;
    } catch {
      return null;
    }
  }

  /** Codex's 5h window: the daemon's on-disk report first, a live session's
   *  status line second.
   *
   *  The order is not a preference between two equal sources. The report
   *  answers with no Codex session running, which is the cold-start case the
   *  status-line path returns null for — and "unknown" there means keep-warm
   *  pings a window that may already be open. The status line is still
   *  consulted after a LAPSED disk reading because it only ever reports a
   *  window that is still running: it can add a window the newest rollout has
   *  not recorded yet, and it can never take one away. */
  private async codexWindow(nowMs: number): Promise<AccountUsageWire | null> {
    const report = await this.fetchUsageReport();
    const onDisk = report ? fiveHourWindowFromReport(report, 'codex', nowMs) : null;
    if (onDisk && windowActive(onDisk, nowMs)) return onDisk;
    // `onDisk` here is either `{}` (definitely lapsed) or null (unreadable);
    // both let a live status line speak, and the difference between them is
    // what evaluate() logs.
    return this.codexWindowFromSessions(nowMs) ?? onDisk;
  }

  /** The freshest 5h reset from live Codex sessions' status lines. Only ever
   *  reports a window that is still running — a session with a lapsed window
   *  is skipped, not reported as expired. */
  private codexWindowFromSessions(nowMs: number): AccountUsageWire | null {
    let best: number | null = null;
    for (const snap of claudeSessionStore.getAllSnapshots()) {
      // Federation: a remote session's rate-limit window belongs to the PEER
      // machine's account — it must not drive the local keep-warm pinger.
      if (snap.provider !== 'codex' || snap.hub) continue;
      const resets = snap.statusLine?.fiveHourResetsAt;
      if (resets != null && resets * 1000 > nowMs && (best == null || resets > best)) {
        best = resets;
      }
    }
    return best != null ? { five_hour_resets_at: best } : null;
  }

  /** One claudemon heartbeat — a minimal turn run by the daemon. */
  private async ping(provider: string, now: Date): Promise<void> {
    const state = this.stateFor(provider);
    console.log(`[keepWarm] requesting ${provider} heartbeat from claudemon`);
    let ok = false;
    let resetsAtSec: number | null = null;
    try {
      const body: Record<string, unknown> = { provider, argv: this.argvFor(provider) };
      if (provider === 'claude') body.model = 'haiku';
      const res = await fetch(`${CLAUDEMON_API_URL}/heartbeat`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
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
        if (!row.ok) {
          console.log(`[keepWarm] ${provider} heartbeat failed: ${row.error ?? 'unknown'}`);
        }
      } else {
        console.log(`[keepWarm] heartbeat endpoint returned ${res.status}`);
      }
    } catch (err) {
      console.log(`[keepWarm] heartbeat request failed: ${(err as Error).message}`);
    }
    if (!ok) {
      state.notBeforeMs = now.getTime() + FAILURE_BACKOFF_MS;
      return;
    }
    state.notBeforeMs = null;
    const resetsAtMs = resetsAtSec != null ? resetsAtSec * 1000 : now.getTime() + FIVE_HOURS_MS;
    state.assumedResetsAtMs = resetsAtMs;
    console.log(
      `[keepWarm] ${provider} window started — resets at ${new Date(resetsAtMs).toLocaleTimeString()}`,
    );
  }

  private argvFor(provider: string): string[] {
    if (provider === 'claude') return claudeBaseArgv();
    const customBin = configService.getConfig().agents?.binaries?.codex;
    return [resolveAgentBinary('codex', customBin)];
  }
}

export const keepWarmService = new KeepWarmService();
