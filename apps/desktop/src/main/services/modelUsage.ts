// Model pricing + context-window lookup for per-agent usage tracking.
//
// The numbers come straight out of the Claude Code transcript JSONL: every
// assistant message carries a `usage` block (input / output / cache tokens)
// and a `model` id. We turn that into two things:
//   • context-window fullness — a point-in-time snapshot of the latest turn
//   • cumulative cost — summed per turn over the session's life
//
// Prices are USD per million tokens. They're estimates and easy to tweak in
// one place if rates change.

export interface RawUsage {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

export interface SessionUsage {
  model: string | null;
  contextTokens: number; // latest turn's input side (point-in-time)
  contextLimit: number; // tokens the model's window holds
  totalInputTokens: number; // cumulative (incl. cache), for cost
  totalOutputTokens: number; // cumulative
  costUSD: number; // cumulative
}

interface ModelRates {
  /** USD per million input tokens. cache-write = 1.25×, cache-read = 0.1×. */
  input: number;
  output: number;
  /** Standard context window in tokens. 1M variants are detected at runtime. */
  contextLimit: number;
}

const DEFAULT_RATES: ModelRates = { input: 3, output: 15, contextLimit: 200_000 };

// Keyed by a prefix of the transcript `model` id (e.g. "claude-opus-4-8").
// Matched longest-prefix-first so "claude-opus-4-8" wins over "claude-opus".
const MODEL_RATES: Record<string, ModelRates> = {
  'claude-opus': { input: 15, output: 75, contextLimit: 200_000 },
  'claude-sonnet': { input: 3, output: 15, contextLimit: 200_000 },
  'claude-haiku': { input: 1, output: 5, contextLimit: 200_000 },
};

function ratesFor(model: string | null | undefined): ModelRates {
  if (!model) return DEFAULT_RATES;
  let best: ModelRates | null = null;
  let bestLen = -1;
  for (const [prefix, rates] of Object.entries(MODEL_RATES)) {
    if (model.startsWith(prefix) && prefix.length > bestLen) {
      best = rates;
      bestLen = prefix.length;
    }
  }
  return best ?? DEFAULT_RATES;
}

/** Tokens occupying the context window on this turn (input + both cache tiers). */
export function contextTokensOf(usage: RawUsage): number {
  return (
    (usage.input_tokens ?? 0) +
    (usage.cache_creation_input_tokens ?? 0) +
    (usage.cache_read_input_tokens ?? 0)
  );
}

// The transcript `model` id doesn't carry the `[1m]` suffix, so we can't read
// the window size off the name. Instead: once a turn's context exceeds the
// standard 200k window, the session must be running in 1M mode — promote the
// limit. (observed is the live contextTokens for this session.)
export function contextLimitFor(model: string | null | undefined, observed: number): number {
  const base = ratesFor(model).contextLimit;
  if (base <= 200_000 && observed > 200_000) return 1_000_000;
  return base;
}

/** USD cost of a single turn. Cache writes cost 1.25× input, reads 0.1×. */
export function turnCostUSD(model: string | null | undefined, usage: RawUsage): number {
  const r = ratesFor(model);
  const cacheWrite = r.input * 1.25;
  const cacheRead = r.input * 0.1;
  const dollars =
    (usage.input_tokens ?? 0) * r.input +
    (usage.cache_creation_input_tokens ?? 0) * cacheWrite +
    (usage.cache_read_input_tokens ?? 0) * cacheRead +
    (usage.output_tokens ?? 0) * r.output;
  return dollars / 1_000_000;
}

export function emptyUsage(): SessionUsage {
  return {
    model: null,
    contextTokens: 0,
    contextLimit: 200_000,
    totalInputTokens: 0,
    totalOutputTokens: 0,
    costUSD: 0,
  };
}
