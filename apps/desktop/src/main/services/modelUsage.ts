// Model pricing + context-window lookup for per-agent usage tracking.
//
// The numbers come straight out of the Claude Code transcript JSONL: every
// assistant message carries a `usage` block (input / output / cache tokens)
// and a `model` id. We turn that into two things:
//   • context-window fullness — a point-in-time snapshot of the latest turn
//   • cumulative cost — summed per turn over the session's life
//
// Prices are USD per million tokens. They're estimates and easy to tweak in
// one place if rates change — or overridden per-machine via the user rates
// file (see MODEL_RATE_OVERRIDES_PATH), which the claudemon Rust engine also
// reads, so one file feeds both costing paths.

import fs from 'fs';
import os from 'os';
import path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';

export interface RawUsage {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

/** Cumulative tokens/cost attributed to one model within a session. */
export interface ModelUsageSlice {
  inputTokens: number; // cumulative incl. cache tiers
  outputTokens: number;
  costUSD: number;
}

export interface SessionUsage {
  model: string | null;
  contextTokens: number; // latest turn's input side (point-in-time)
  contextLimit: number; // tokens the model's window holds
  totalInputTokens: number; // cumulative (incl. cache), for cost
  totalOutputTokens: number; // cumulative
  costUSD: number; // cumulative
  /** Per-model split of the cumulative figures — main thread and subagent
   *  (sidechain) turns alike, keyed by concrete model id. */
  models: Record<string, ModelUsageSlice>;
}

interface ModelRates {
  /** USD per million input tokens. cache-write = 1.25×, cache-read = 0.1×. */
  input: number;
  output: number;
  /** USD per million cache-read tokens. Undefined ⇒ 0.1× input (the built-in
   *  default). Mirrors the Rust engine's `cached_input.unwrap_or(input*0.1)`. */
  cachedInput?: number;
  /** Standard context window in tokens. 1M variants are detected at runtime. */
  contextLimit: number;
}

export const DEFAULT_RATES: ModelRates = { input: 3, output: 15, contextLimit: 200_000 };

// Keyed by a prefix of the transcript `model` id (e.g. "claude-opus-4-8").
// Matched longest-prefix-first so "claude-opus-4-1-" wins over "claude-opus".
// Current list pricing (2026-06): Fable $10/$50, Opus 4.5+ $5/$25,
// Sonnet $3/$15, Haiku $1/$5. Opus 4.1 and older kept the $15/$75 rates.
//
// NOTE on the legacy Opus keys: transcripts carry the *dated* ids. Opus 4.0 is
// 'claude-opus-4-20250514' — which does NOT start with the 'claude-opus-4-0'
// alias — so it needs its own 'claude-opus-4-20' prefix (dated 4.0 ids are the
// only ids that continue "4-2" with a "0"). Opus 4.1 is
// 'claude-opus-4-1-20250805', so the key carries a trailing '-'
// ('claude-opus-4-1-') to pin it to the 4.1 generation only — a bare
// 'claude-opus-4-1' prefix would also swallow 4.10–4.19 ('claude-opus-4-10…'),
// which are current generations and should price at the generic 5/25.
// Claude 3 Opus ids ('claude-3-opus-20240229') don't start with 'claude-opus'
// at all, hence the separate 'claude-3-opus' entry.
export const MODEL_RATES: Record<string, ModelRates> = {
  // Fable / Mythos are 1M-native (no [1m] id marker to detect at runtime) —
  // the 200K default made their context gauges read 5× too high.
  'claude-fable': { input: 10, output: 50, contextLimit: 1_000_000 },
  'claude-mythos': { input: 10, output: 50, contextLimit: 1_000_000 },
  'claude-opus': { input: 5, output: 25, contextLimit: 200_000 },
  'claude-opus-4-1-': { input: 15, output: 75, contextLimit: 200_000 },
  'claude-opus-4-0': { input: 15, output: 75, contextLimit: 200_000 },
  'claude-opus-4-20': { input: 15, output: 75, contextLimit: 200_000 },
  'claude-3-opus': { input: 15, output: 75, contextLimit: 200_000 },
  'claude-sonnet': { input: 3, output: 15, contextLimit: 200_000 },
  'claude-haiku': { input: 1, output: 5, contextLimit: 200_000 },
};

// ── User rate overrides ──────────────────────────────────────────────────────
// The same file the claudemon Rust engine (session/pricing.rs) reads, so a rate
// edited in Settings applies to both costing paths. Keys are model-id prefixes;
// snake_case fields (`cached_input`, `context_limit`) match the Rust reader.
export const MODEL_RATE_OVERRIDES_PATH = path.join(os.homedir(), '.workspacer', 'model-rates.json');

export interface ModelRateOverride {
  input: number;
  output: number;
  cached_input?: number;
  context_limit?: number;
}
export type ModelRateOverrides = Record<string, ModelRateOverride>;

// mtime-keyed cache so the hot paths (per-turn costing) cost one stat, not a
// parse — mirrors pricing.rs's OVERRIDES cache.
let overridesCache: { mtimeMs: number | null; table: ModelRateOverrides } | null = null;

function loadOverrides(): ModelRateOverrides {
  let mtimeMs: number | null = null;
  try {
    mtimeMs = fs.statSync(MODEL_RATE_OVERRIDES_PATH).mtimeMs;
  } catch {
    mtimeMs = null; // no file (or unreadable) — built-ins only
  }
  if (overridesCache && overridesCache.mtimeMs === mtimeMs) return overridesCache.table;
  let table: ModelRateOverrides = {};
  if (mtimeMs !== null) {
    try {
      const parsed = JSON.parse(fs.readFileSync(MODEL_RATE_OVERRIDES_PATH, 'utf-8'));
      if (parsed && typeof parsed === 'object') table = parsed as ModelRateOverrides;
    } catch {
      table = {}; // invalid JSON — ignore, built-ins only
    }
  }
  overridesCache = { mtimeMs, table };
  return table;
}

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
  // User overrides win on the longest prefix (`>=` so a same-length override
  // beats the built-in). A valid override carries input + output; context_limit
  // is optional and falls back to the built-in/default window.
  for (const [prefix, ov] of Object.entries(loadOverrides())) {
    if (
      model.startsWith(prefix) &&
      prefix.length >= bestLen &&
      typeof ov?.input === 'number' &&
      typeof ov?.output === 'number'
    ) {
      best = {
        input: ov.input,
        output: ov.output,
        // Honor an optional cache-read override — the Rust engine (usage.rs
        // turn_cost_usd) reads the same field, so dropping it here diverges the
        // two costing paths for the same model-rates.json.
        cachedInput: typeof ov.cached_input === 'number' ? ov.cached_input : undefined,
        // A window is a token COUNT: accept it only as a non-negative integer,
        // matching the Rust reader's `as_u64` (pricing.rs parse_one_override).
        // A fractional or negative value is treated as absent so both costing
        // paths inherit the same matched/default window for the same file.
        contextLimit:
          typeof ov.context_limit === 'number' &&
          Number.isInteger(ov.context_limit) &&
          ov.context_limit >= 0
            ? ov.context_limit
            : (best?.contextLimit ?? DEFAULT_RATES.contextLimit),
      };
      bestLen = prefix.length;
    }
  }
  return best ?? DEFAULT_RATES;
}

/** Current overrides on disk (for the Settings editor). */
export function readModelRateOverrides(): ModelRateOverrides {
  return loadOverrides();
}

/** Persist overrides. An empty map deletes the file (revert to built-ins). */
export function writeModelRateOverrides(overrides: ModelRateOverrides): void {
  if (!overrides || Object.keys(overrides).length === 0) {
    try {
      fs.rmSync(MODEL_RATE_OVERRIDES_PATH);
    } catch {
      /* already absent */
    }
  } else {
    fs.mkdirSync(path.dirname(MODEL_RATE_OVERRIDES_PATH), { recursive: true });
    atomicWriteFileSync(MODEL_RATE_OVERRIDES_PATH, JSON.stringify(overrides, null, 2));
  }
  overridesCache = null; // force a re-read on next costing call
}

/** Tokens occupying the context window on this turn (input + both cache tiers). */
export function contextTokensOf(usage: RawUsage): number {
  return (
    (usage.input_tokens ?? 0) +
    (usage.cache_creation_input_tokens ?? 0) +
    (usage.cache_read_input_tokens ?? 0)
  );
}

/** Extra truth about a session's window, beyond its transcript `model` id.
 *  Both are optional — a session that has neither falls back to the table plus
 *  the retrospective promotion below. */
export interface ContextLimitHints {
  /** The window the provider itself reported, in tokens. For Claude this is
   *  `context_window.context_window_size` off the statusLine payload (PTY) or
   *  the stream `result` frame's `modelUsage.*.contextWindow`; for Codex it's
   *  `model_context_window`. Real provider truth, so it outranks every guess. */
  reportedWindow?: number | null;
  /** The model string this session was *asked* for at spawn — the alias the
   *  user picked in the composer (`opus[1m]`), not the concrete id. Claude
   *  Code strips the `[1m]` marker from the transcript's `model` field, so
   *  this is the only place the 1M choice survives before the provider has
   *  reported a window of its own. */
  requestedModel?: string | null;
}

/** 1M window implied by a *requested* model string, if it names one.
 *  `[1m]` is Claude Code's own marker (`opus[1m]`, `claude-opus-5[1m]`) and
 *  `-1m` is the id-suffix spelling; Fable/Mythos are 1M-native and carry no
 *  marker at all. Anything else returns undefined — an unmarked alias says
 *  nothing about the window, it does NOT mean 200k. Mirrors the Rust twin
 *  `providers::context_window_for`. */
export function requestedWindowFor(model: string | null | undefined): number | undefined {
  if (!model) return undefined;
  const m = model.toLowerCase();
  if (m.includes('[1m]') || m.includes('-1m')) return 1_000_000;
  if (m.includes('fable') || m.includes('mythos')) return 1_000_000;
  return undefined;
}

/**
 * The window this session's context bar should be drawn against.
 *
 * Resolved best-truth-first, because the transcript's `model` id alone cannot
 * answer the question — Claude Code strips the `[1m]` marker from it, so a 1M
 * session and a 200k session report the same id:
 *
 *   1. the window the provider reported (`hints.reportedWindow`) — a fact;
 *   2. the model requested at spawn (`hints.requestedModel`), which still has
 *      its `[1m]` marker — known from token zero, and never allowed to *lower*
 *      the table's window;
 *   3. the rates table for the transcript model id (incl. user overrides);
 *   4. last resort — the retrospective promotion: once a turn's context has
 *      exceeded the standard 200k window the session must be in 1M mode. Kept
 *      because it is the only signal for a session whose window is genuinely
 *      unknown (an adopted/foreign session with no spawn record and no
 *      statusLine yet), but demoted below the two real signals: it only
 *      corrects itself at the point the reading stops being useful.
 *
 * `observed` is the session's high-water contextTokens.
 */
export function contextLimitFor(
  model: string | null | undefined,
  observed: number,
  hints?: ContextLimitHints,
): number {
  const reported = hints?.reportedWindow;
  if (typeof reported === 'number' && Number.isFinite(reported) && reported > 0) return reported;
  const base = ratesFor(model).contextLimit;
  const requested = requestedWindowFor(hints?.requestedModel);
  // `max`, never assignment: a user `context_limit` override (or a 1M-native
  // model id) must not be dragged down by a coarser requested alias.
  if (requested !== undefined) return Math.max(base, requested);
  if (base <= 200_000 && observed > 200_000) return 1_000_000;
  return base;
}

/** USD cost of a single turn. Cache writes cost 1.25× input, reads 0.1×. */
export function turnCostUSD(model: string | null | undefined, usage: RawUsage): number {
  const r = ratesFor(model);
  const cacheWrite = r.input * 1.25;
  // Cache reads default to 0.1× input, but a user override (cached_input) wins —
  // matching the Rust engine's `cached_input.unwrap_or(input * 0.1)`.
  const cacheRead = typeof r.cachedInput === 'number' ? r.cachedInput : r.input * 0.1;
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
    models: {},
  };
}
