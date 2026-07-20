/**
 * Consumes the daemon's `/statusline/stream` SSE feed and forwards each tick
 * into `claudeSessionStore.applyStatusLine`. This is a separate channel from
 * `/hooks/stream` because the statusLine command fires very frequently — it is
 * deliberately kept off the hook fanout (and thus off the SQLite persistence
 * path). Reconnects with exponential backoff if the daemon restarts.
 */

import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';

let abort: AbortController | null = null;

// NOTE: this bridge deliberately raises NO alert for `rate_limit_warning`.
// It used to fire an OS notification ("Approaching a usage limit") plus an
// in-app banner at the daemon's 80% threshold — an interruption built around a
// coarse yes/no flag, from back when that flag was the only usage signal we
// had. The per-window gauges (5h / 7d / monthly `% used` with reset times, in
// the status bar and Inspector) are strictly more accurate and always on
// screen, so the warning is now carried as passive text on the snapshot
// (`rateLimitWarning`, rendered by InspectorCard) and never interrupts.

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
