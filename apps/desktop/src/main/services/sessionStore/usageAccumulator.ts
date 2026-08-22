import { contextTokensOf, contextLimitFor, turnCostUSD, emptyUsage } from '../modelUsage';
import { configService } from '../configService';
import type { ClaudeSessionState } from '../claudeSessionStore';

// ── SessionUsageAccumulator ───────────────────────────────────────────────────

export class SessionUsageAccumulator {
  // sessionId → set of accounted assistant message ids, so re-seen transcript
  // lines (streamed blocks of one message AND full replays during a conversation
  // resync/reset) don't double-count cumulative cost / token totals.
  private seenKeys = new Map<string, Set<string>>();
  // Concrete model ids we've already persisted to config, so we only write on
  // genuinely-new models. Lazily seeded from config on first use.
  private knownModels: Set<string> | null = null;

  /**
   * Fold one assistant message's `usage` into the session.
   * Context = latest turn's input side (overwritten each time, idempotent).
   * Totals/cost accumulate, deduped by message id so streamed blocks of the
   * same message aren't counted twice.
   *
   * `sidechain` marks a subagent (isSidechain) turn: its tokens/cost count
   * toward the session totals and the per-model split — priced at the
   * subagent's own model rates — but it must not move the main thread's
   * context gauge or reported model.
   */
  applyUsage(
    session: ClaudeSessionState,
    model: string | null,
    usage: any,
    key: string | null,
    sidechain = false,
  ): void {
    if (!session.usage) session.usage = emptyUsage();
    const u = session.usage;
    if (!u.models) u.models = {}; // sessions restored from pre-split snapshots

    if (model) this.rememberModel(model);
    if (!sidechain) {
      const ctx = contextTokensOf(usage);
      u.contextTokens = ctx;
      if (ctx > session.peakContext) session.peakContext = ctx;
      if (model) u.model = model;
      SessionUsageAccumulator.refreshContextLimit(session);
    }

    // Cumulative — only once per distinct message, ever (idempotent under
    // replay, not just consecutive dedup).
    if (key) {
      let seen = this.seenKeys.get(session.sessionId);
      if (!seen) this.seenKeys.set(session.sessionId, (seen = new Set()));
      if (seen.has(key)) return;
      seen.add(key);
    }
    const turnModel = model ?? u.model;
    const inputTokens = contextTokensOf(usage);
    const outputTokens = usage.output_tokens ?? 0;
    const costUSD = turnCostUSD(turnModel, usage);
    u.totalInputTokens += inputTokens;
    u.totalOutputTokens += outputTokens;
    u.costUSD += costUSD;

    const slice = (u.models[turnModel ?? '(unknown)'] ??= {
      inputTokens: 0,
      outputTokens: 0,
      costUSD: 0,
    });
    slice.inputTokens += inputTokens;
    slice.outputTokens += outputTokens;
    slice.costUSD += costUSD;
  }

  /**
   * Recompute `usage.contextLimit` from everything the session currently knows
   * about its window. Static because it depends on no accumulator state, and
   * called from three places, not one: a usage turn (below), the statusLine
   * (which carries the provider's *real* window) and a spawn/model-switch
   * (which carries the requested `opus[1m]` alias). Either of those can land
   * after the last usage item, and until it is folded in the bar is drawn
   * against a stale denominator.
   *
   * No-op when the session has no usage yet — an all-zero gauge renders
   * nothing, so there is no wrong number to correct.
   */
  static refreshContextLimit(session: ClaudeSessionState): void {
    const u = session.usage;
    if (!u) return;
    // The high-water mark, not just this turn: the retrospective fallback is a
    // session-level verdict, so once any turn has exceeded the 200k window the
    // limit stays promoted even when a later turn's context is smaller.
    u.contextLimit = contextLimitFor(u.model, session.peakContext, {
      reportedWindow: session.statusLine?.contextWindowSize,
      requestedModel: session.settings?.model,
    });
  }

  /** Remove all per-session state for a session that has been evicted. */
  forget(sessionId: string): void {
    this.seenKeys.delete(sessionId);
  }

  /** Persist a concrete model id to config the first time we see it, so the
   *  spawn dropdown can offer it across restarts. */
  private rememberModel(model: string): void {
    // `<synthetic>` is Claude Code's placeholder id on synthetic messages, not
    // a launchable model — keep it out of the persisted picker list.
    if (model.startsWith('<')) return;
    if (this.knownModels === null) {
      const cfg = configService.getConfig() as any;
      this.knownModels = new Set(
        Array.isArray(cfg.claude?.seenModels) ? cfg.claude.seenModels : [],
      );
    }
    if (this.knownModels.has(model)) return;
    this.knownModels.add(model);
    // Re-read the on-disk set at write time and union it in: another writer
    // (brain / web / mobile) may have appended models to seenModels since we
    // cached, and deepMerge replaces arrays wholesale — persisting only our
    // stale cache would clobber those external additions (the exact 'settings
    // getting reset' the mtime gate exists to prevent).
    const fresh = configService.getConfig() as any;
    const onDisk: string[] = Array.isArray(fresh.claude?.seenModels) ? fresh.claude.seenModels : [];
    for (const m of onDisk) this.knownModels.add(m);
    configService.saveConfig({
      claude: { seenModels: Array.from(this.knownModels).sort() },
    } as any);
  }
}
