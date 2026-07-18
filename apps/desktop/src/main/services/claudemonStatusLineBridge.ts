/**
 * Consumes the daemon's `/statusline/stream` SSE feed and forwards each tick
 * into `claudeSessionStore.applyStatusLine`. This is a separate channel from
 * `/hooks/stream` because the statusLine command fires very frequently — it is
 * deliberately kept off the hook fanout (and thus off the SQLite persistence
 * path). Reconnects with exponential backoff if the daemon restarts.
 */

import { Notification } from 'electron';
import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';
import { configService } from './configService';
import { notifySystem } from './systemNotice';

let abort: AbortController | null = null;

// Rate-limit warnings are per-ACCOUNT but arrive on every session's tick, and
// Claude vs Codex are DISTINCT accounts sharing this one feed. So we dedup by
// provider (the account proxy) × window — a climbing % on the same window must
// not re-fire, but a Claude 5h warning must NOT swallow a Codex 5h warning.
// Each provider's entry is cleared when that account is comfortable again so the
// next episode re-alerts.
const lastWarnedWindow = new Map<string, string>();

/** The daemon raises/clears the window warning at this utilization %. */
const RATE_LIMIT_WARN_THRESHOLD = 80;

/** Coarse window key ('5h' | '7d' | 'monthly') from a warning message. */
function windowKeyOf(msg: string): string {
  if (msg.includes('7-day')) return '7d';
  if (msg.includes('monthly')) return 'monthly';
  return '5h';
}

/** Fire a rate-limit warning once per window-warning episode: an OS
 *  notification (respecting the master switch + sound) plus an in-app banner.
 *  `pcts` carries the window gauges so a warning-less frame can be judged. */
function raiseRateLimitWarning(
  provider: string,
  message: string | undefined,
  pcts?: { fiveHour?: number; sevenDay?: number; monthly?: number },
): void {
  const warned = lastWarnedWindow.get(provider) ?? null;
  if (!message) {
    // A warning-less frame does NOT prove the account is comfortable:
    // interactive (PTY) sessions never carry a warning even at high
    // utilization, and the periodic account-usage re-push is warning-less too.
    // Only re-arm the alert once the *warned* window's own gauge is back under
    // the daemon's threshold — mirroring how the daemon clears the warning.
    if (warned) {
      const pct =
        warned === '7d' ? pcts?.sevenDay : warned === 'monthly' ? pcts?.monthly : pcts?.fiveHour;
      if (typeof pct === 'number' && pct < RATE_LIMIT_WARN_THRESHOLD) {
        lastWarnedWindow.delete(provider); // comfortable again → allow the next episode to alert
      }
    }
    return;
  }
  const key = windowKeyOf(message);
  if (key === warned) return; // same window still warning for this account — no re-fire
  lastWarnedWindow.set(provider, key);

  const cfg = (configService.getConfig() as any).notifications ?? {};
  const enabled = cfg.enabled !== false;
  if (enabled && Notification.isSupported()) {
    new Notification({
      title: 'Approaching a usage limit',
      body: message,
      silent: cfg.sound !== true,
    }).show();
  }
  // The in-app banner always shows (it's the app's own surface, not an OS
  // notification); a stable key means a later window replaces rather than stacks.
  notifySystem({ level: 'warn', key: 'rate-limit-warning', title: message });
}

/** Map the wire's snake_case context inventory to the store's camelCase shape. */
function mapInventory(inv: any): import('./claudeSessionStore').ContextInventoryInfo | undefined {
  if (!inv) return undefined;
  const items = (list: any): any[] =>
    Array.isArray(list)
      ? list.map((i: any) => ({
          name: i.name,
          path: i.path,
          status: i.status,
          source: i.source,
          bytes: i.bytes,
          estTokens: i.est_tokens,
        }))
      : [];
  return {
    mcpServers: items(inv.mcp_servers),
    skills: items(inv.skills),
    agents: items(inv.agents),
    plugins: items(inv.plugins),
    memoryFiles: items(inv.memory_files),
    tools: Array.isArray(inv.tools) ? inv.tools : [],
    slashCommands: Array.isArray(inv.slash_commands) ? inv.slash_commands : [],
    claudeCodeVersion: inv.claude_code_version,
  };
}

export async function startClaudemonStatusLineBridge(): Promise<void> {
  // Idempotent: if already running, skip re-starting.
  if (abort) return;
  abort = new AbortController();
  const url = `${CLAUDEMON_API_URL}/statusline/stream`;
  console.log(`[claudemon-statusline] subscribed to ${url}`);
  await consumeSseStream(url, {
    signal: abort.signal,
    backoffInitialMs: 200,
    backoffMaxMs: 5000,
    joinWith: '\n',
    onFrame(dataString) {
      let update: any;
      try {
        update = JSON.parse(dataString);
      } catch (err) {
        console.warn('[claudemon-statusline] malformed JSON frame, skipping:', err);
        return;
      }
      try {
        const sl = update.status_line ?? {};
        // Map the wire (snake_case) shape to the renderer's camelCase type.
        claudeSessionStore.applyStatusLine(update.session_id, {
          modelDisplay: sl.model_display,
          contextUsedPct: sl.context_used_pct,
          contextWindowSize: sl.context_window_size,
          totalInputTokens: sl.total_input_tokens,
          totalOutputTokens: sl.total_output_tokens,
          costUSD: sl.cost_usd,
          fiveHourPct: sl.five_hour_pct,
          fiveHourResetsAt: sl.five_hour_resets_at,
          sevenDayPct: sl.seven_day_pct,
          sevenDayResetsAt: sl.seven_day_resets_at,
          monthlyPct: sl.monthly_pct,
          monthlyResetsAt: sl.monthly_resets_at,
          rateLimitWarning: sl.rate_limit_warning,
          overageOutOfCredits: sl.overage_out_of_credits,
          capabilities: sl.capabilities
            ? {
                fastMode: sl.capabilities.fast_mode,
                outputStyle: sl.capabilities.output_style,
                apiKeySource: sl.capabilities.api_key_source,
                mcpServers: sl.capabilities.mcp_servers,
                skills: sl.capabilities.skills,
                plugins: sl.capabilities.plugins,
                agents: sl.capabilities.agents,
                memoryFiles: sl.capabilities.memory_files,
                inventory: mapInventory(sl.capabilities.inventory),
              }
            : undefined,
          receivedAt: sl.received_at,
        });
        // Windows are per-account; key the dedup latch on the session's provider
        // (Claude and Codex are distinct accounts sharing this one feed).
        const provider = claudeSessionStore.providerOf?.(update.session_id) ?? 'claude';
        raiseRateLimitWarning(provider, sl.rate_limit_warning, {
          fiveHour: sl.five_hour_pct,
          sevenDay: sl.seven_day_pct,
          monthly: sl.monthly_pct,
        });
      } catch (err) {
        console.error('[claudemon-statusline] bad frame', err);
      }
    },
    onError(err) {
      console.warn('[claudemon-statusline] stream error, retrying:', err);
    },
  });
}

export function stopClaudemonStatusLineBridge(): void {
  if (abort) {
    try {
      abort.abort();
    } catch {}
    abort = null;
  }
}
