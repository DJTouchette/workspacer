import {
  contextTokensOf,
  contextLimitFor,
  turnCostUSD,
  cacheSplitOf,
  emptyUsage,
} from '../modelUsage';
import { configService } from '../configService';
import type { PendingReadOnlySession } from './pendingSlot';

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
    session: PendingReadOnlySession,
    model: string | null,
    usage: any,
    key: string | null,
    sidechain = false,
  ): void {
    if (!session.usage) session.usage = emptyUsage();
    const u = session.usage;
    if (!u.models) u.models = {}; // sessions restored from pre-split snapshots

    // `<synthetic>` is Claude Code's placeholder id on rows the CLI generates
    // locally rather than from a model — notably its "No response requested."
    // reply to a resume kickoff, which carries all-zero usage. That is not an
    // observation of the context window and not a model that ran, so folding it
    // in ZEROES the gauge and renames the session's model, manufacturing a
    // freeze-shaped reading on a perfectly healthy session (it did exactly that
    // during the 2026-08-23 wedge investigation). Same `<`-prefix predicate
    // `rememberModel` below already applies; the Rust reader guards its own
    // fold the same way (`session/usage.rs`).
    const placeholder = !!model && model.startsWith('<');

    if (model) this.rememberModel(model);
    if (!sidechain && !placeholder) {
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
    // A placeholder row still counts (its zeros are harmless), but it is priced
    // and filed under the thread's model — never as a `<synthetic>` slice of
    // its own. Parity with the Rust reader's `row_model.or(usage.model)`.
    const turnModel = (placeholder ? null : model) ?? u.model;
    const inputTokens = contextTokensOf(usage);
    const outputTokens = usage.output_tokens ?? 0;
    // The three prompt tiers, kept apart instead of only summed. `inputTokens`
    // above is their total and stays the number every existing surface reads;
    // this is the split behind it, and it exists only once a provider has
    // actually reported cache fields (see cacheSplitOf: null, not zeros).
    const split = cacheSplitOf(usage);
    if (split) {
      const c = (u.cache ??= { fresh: 0, write: 0, read: 0 });
      c.fresh += split.fresh;
      c.write += split.write;
      c.read += split.read;
    }
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
  static refreshContextLimit(session: PendingReadOnlySession): void {
    const u = session.usage;
    if (!u) return;
    // The high-water mark, not just this turn: the DRIFT ALARM is a
    // session-level verdict. Once this session has been seen holding more than
    // the window we claim, that claim is disproved for good — auto-compaction
    // dropping the latest turn back under the line does not re-prove it.
    // (This used to be the retrospective 200k→1M PROMOTION, which read the same
    // high-water mark and drew a much stronger conclusion from it.)
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
