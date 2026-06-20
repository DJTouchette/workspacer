import {
  contextTokensOf,
  contextLimitFor,
  turnCostUSD,
  emptyUsage,
} from '../modelUsage';
import { configService } from '../configService';
import type { ClaudeSessionState } from '../claudeSessionStore';

// ── SessionUsageAccumulator ───────────────────────────────────────────────────

export class SessionUsageAccumulator {
  // sessionId → last accounted assistant message id, so re-seen transcript
  // lines (blocks of one message stream as separate entries) don't double-count
  // cumulative cost / token totals.
  private lastUsageKey = new Map<string, string>();
  // Concrete model ids we've already persisted to config, so we only write on
  // genuinely-new models. Lazily seeded from config on first use.
  private knownModels: Set<string> | null = null;

  /**
   * Fold one assistant message's `usage` into the session.
   * Context = latest turn's input side (overwritten each time, idempotent).
   * Totals/cost accumulate, deduped by message id so streamed blocks of the
   * same message aren't counted twice.
   */
  applyUsage(
    session: ClaudeSessionState,
    model: string | null,
    usage: any,
    key: string | null,
  ): void {
    if (!session.usage) session.usage = emptyUsage();
    const u = session.usage;

    const ctx = contextTokensOf(usage);
    u.contextTokens = ctx;
    if (ctx > session.peakContext) session.peakContext = ctx;
    if (model) {
      u.model = model;
      this.rememberModel(model);
    }
    u.contextLimit = contextLimitFor(u.model, ctx);

    // Cumulative — only once per distinct message.
    if (key && this.lastUsageKey.get(session.sessionId) === key) return;
    if (key) this.lastUsageKey.set(session.sessionId, key);
    u.totalInputTokens += ctx;
    u.totalOutputTokens += usage.output_tokens ?? 0;
    u.costUSD += turnCostUSD(u.model, usage);
  }

  /** Remove all per-session state for a session that has been evicted. */
  forget(sessionId: string): void {
    this.lastUsageKey.delete(sessionId);
  }

  /** Persist a concrete model id to config the first time we see it, so the
   *  spawn dropdown can offer it across restarts. */
  private rememberModel(model: string): void {
    if (this.knownModels === null) {
      const cfg = configService.getConfig() as any;
      this.knownModels = new Set(Array.isArray(cfg.claude?.seenModels) ? cfg.claude.seenModels : []);
    }
    if (this.knownModels.has(model)) return;
    this.knownModels.add(model);
    configService.saveConfig({ claude: { seenModels: Array.from(this.knownModels).sort() } } as any);
  }
}
