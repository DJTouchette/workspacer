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
import { resolveContextWindow } from '../shared/modelContextWindows';
import os from 'os';
import path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';

/** The TTL split Claude reports alongside `cache_creation_input_tokens`.
 *  Each cache write is tagged with the lifetime it was written for, and the
 *  two lifetimes bill at different multiples of the base input rate. */
export interface RawCacheCreation {
  ephemeral_5m_input_tokens?: number;
  ephemeral_1h_input_tokens?: number;
}

export interface RawUsage {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
  /** Per-TTL breakdown of `cache_creation_input_tokens`. Present on every
   *  modern Claude transcript turn that wrote to cache; absent on older ones
   *  and on providers that do not itemize writes. */
  cache_creation?: RawCacheCreation;
}

/** How a session's cumulative prompt tokens divided between fresh input, cache
 *  writes and cache reads.
 *
 *  Present only once a provider has actually reported cache fields. Absence
 *  means "not reported", never "zero". A provider that itemizes nothing must
 *  not be drawn as one that cached nothing. */
export interface CacheTokenSplit {
  /** Prompt tokens processed fresh, at the full input rate. */
  fresh: number;
  /** Prompt tokens written into the cache this session. */
  write: number;
  /** Prompt tokens served back from the cache. */
  read: number;
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
  /** Tokens the model's window holds, or `null` when we DO NOT KNOW.
   *
   *  Unknown is a real answer and it is spelled `null`, never a guessed
   *  `200_000` — which is how four sites used to spell it, `emptyUsage()`
   *  included, so a pane asserted a 200k window before a single token had been
   *  counted. Every readout already hides its meter on a falsy limit; this is
   *  what finally lets them. Resolved by `resolveContextWindow`, nowhere else. */
  contextLimit: number | null;
  totalInputTokens: number; // cumulative (incl. cache), for cost
  totalOutputTokens: number; // cumulative
  costUSD: number; // cumulative
  /** Per-model split of the cumulative figures — main thread and subagent
   *  (sidechain) turns alike, keyed by concrete model id. */
  models: Record<string, ModelUsageSlice>;
  /** Fresh / cache-write / cache-read split of `totalInputTokens`, cumulative.
   *  Undefined until a turn arrives carrying cache fields. */
  cache?: CacheTokenSplit;
}

interface ModelRates {
  /** USD per million input tokens. Cache writes bill at a multiple of this
   *  (see CACHE_WRITE_5M/1H_MULTIPLIER), cache reads at 0.1×. */
  input: number;
  output: number;
  /** USD per million cache-read tokens. Undefined ⇒ 0.1× input (the built-in
   *  default). Mirrors the Rust engine's `cached_input.unwrap_or(input*0.1)`. */
  cachedInput?: number;
}

/** Sonnet-tier PRICE defaults for a model the table has never heard of.
 *
 *  It used to carry `contextLimit: 200_000` as well — the second of the four
 *  places "we do not know" was spelled as a number. An unknown model now has an
 *  unknown WINDOW (`windowFor` returns null); guessing its PRICE is a different
 *  call, and a defensible one, because a blank cost reads as free rather than
 *  as unknown. */
export const DEFAULT_RATES: ModelRates = { input: 3, output: 15 };

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
//
// This table is PRICE only. It used to carry a `contextLimit` per row, which
// made it one of five parallel window tables — and the only one with no OpenAI
// rows at all, so `contextLimitFor('gpt-5.3-codex')` floored to 200_000 while
// the daemon's provider table said 272_000, for the same session. Windows now
// come from `main/shared/modelContextWindows`, pinned to
// contracts/model-context-windows.json.
export const MODEL_RATES: Record<string, ModelRates> = {
  'claude-fable': { input: 10, output: 50 },
  'claude-mythos': { input: 10, output: 50 },
  'claude-opus': { input: 5, output: 25 },
  'claude-opus-4-1-': { input: 15, output: 75 },
  'claude-opus-4-0': { input: 15, output: 75 },
  'claude-opus-4-20': { input: 15, output: 75 },
  'claude-3-opus': { input: 15, output: 75 },
  'claude-sonnet': { input: 3, output: 15 },
  'claude-haiku': { input: 1, output: 5 },
  // OpenAI. TWIN of pricing.rs's BUILTIN, which has carried these since Codex
  // landed; this side did not, so the same model id priced at DEFAULT_RATES
  // ($3/$15) here and at its real rate in the daemon. GitHub Copilot CLI made
  // that divergence reachable from a second direction — its catalog is
  // multi-vendor (see services/claudemon/src/providers/copilot.rs) — so the
  // two tables are now mirrored and pinned by contracts/model-pricing-cases.json.
  'gpt-5': { input: 1.25, output: 10, cachedInput: 0.125 },
  'gpt-5-codex': { input: 1.25, output: 10, cachedInput: 0.125 },
  'gpt-5-mini': { input: 0.25, output: 2, cachedInput: 0.025 },
  'gpt-5-nano': { input: 0.05, output: 0.4, cachedInput: 0.005 },
  'gpt-5-pro': { input: 15, output: 120 },
  'gpt-4.1': { input: 2, output: 8, cachedInput: 0.5 },
  'codex-mini': { input: 1.5, output: 6, cachedInput: 0.375 },
  o3: { input: 2, output: 8, cachedInput: 0.5 },
  'o4-mini': { input: 1.1, output: 4.4, cachedInput: 0.275 },
  // Copilot's Google (Gemini) and xAI (Grok) models are deliberately ABSENT
  // from both tables — see the note beside pricing.rs's BUILTIN. They fall to
  // DEFAULT_RATES here (this engine only ever sees Claude transcript ids, so
  // that path is unreachable in practice) and to a blank estimate in the
  // daemon, which is the readout a managed session gets.
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

/** The user's EXPLICIT window override for a model, if they wrote one.
 *
 *  A window is a token COUNT: accepted only as a non-negative integer, matching
 *  the Rust reader's `as_u64` (pricing.rs `parse_one_override`), so a fractional
 *  or negative value is treated as absent and both engines resolve the same
 *  window for the same model-rates.json.
 *
 *  This is rank 2 of the hierarchy (see `resolveContextWindow`) — ABOVE the
 *  `[1m]` marker, because a person who writes `context_limit` means it. It used
 *  to be a `Math.max()` against the marker, under which a coarse alias could
 *  silently raise the window back over what the user wrote.
 *
 *  TWIN: `pricing::override_window_for` in services/claudemon. */
export function overrideWindowFor(model: string | null | undefined): number | null {
  if (!model) return null;
  let best: number | null = null;
  let bestLen = -1;
  for (const [prefix, ov] of Object.entries(loadOverrides())) {
    if (
      model.startsWith(prefix) &&
      prefix.length >= bestLen &&
      typeof ov?.input === 'number' &&
      typeof ov?.output === 'number'
    ) {
      best =
        typeof ov.context_limit === 'number' &&
        Number.isInteger(ov.context_limit) &&
        ov.context_limit >= 0
          ? ov.context_limit
          : null;
      bestLen = prefix.length;
    }
  }
  return best;
}

/** Extra truth about a session's window, beyond its transcript `model` id.
 *  All optional — a session with none of them falls back to the contract table,
 *  and then to honest unknown. */
export interface ContextLimitHints {
  /** The window the provider itself reported, in tokens. For Claude this is
   *  `context_window.context_window_size` off the statusLine payload (PTY) or
   *  the stream `result` frame's `modelUsage.*.contextWindow`; for Codex it's
   *  `model_context_window`. Real provider truth, so it outranks every guess. */
  reportedWindow?: number | null;
  /** The model string this session was *asked* for at spawn — the alias the
   *  user picked (`opus[1m]`), not the concrete id. Claude Code strips the
   *  `[1m]` marker from the transcript's `model` field, so this is the only
   *  place the 1M choice survives before the provider has reported a window. */
  requestedModel?: string | null;
}

/**
 * The window this session's context bar should be drawn against, or `null` when
 * we do not know — in which case every readout in the repo already hides the
 * meter rather than drawing one against an invented denominator.
 *
 * A thin call site over `resolveContextWindow`, which owns the hierarchy for
 * all three stacks and is pinned to contracts/model-context-windows.json:
 * provider-reported > user override > requested-model `[1m]` marker > contract
 * table > unknown, then the drift alarm.
 *
 * `observed` is the session's high-water contextTokens, and it is no longer a
 * SOURCE. It used to promote a 200k window to 1M once a turn crossed 200_000,
 * which is a guess dressed as a correction and could only ever fire after ~200k
 * tokens of already-wrong readings; it now DISARMS a window it disproves, so a
 * stale table announces itself instead of lying quietly.
 */
export function contextLimitFor(
  model: string | null | undefined,
  observed: number,
  hints?: ContextLimitHints,
): number | null {
  return resolveContextWindow(model, {
    reportedWindow: hints?.reportedWindow,
    overrideWindow: overrideWindowFor(model),
    requestedModel: hints?.requestedModel,
    peakContext: observed,
  });
}

// ── Cache multipliers ────────────────────────────────────────────────
//
// A cache write costs more than fresh input because the write is kept alive for
// a chosen lifetime, and the price scales with that lifetime: 1.25× the base
// input rate at the 5-minute TTL, 2× at the 1-hour TTL. Reads cost 0.1×.
//
// These were a single hardcoded 1.25× until 2026-08-24, which is the 5-minute
// rate. Workspacer's own sessions are almost entirely 1-hour, so every displayed
// cost charged 1.25× for writes the account was billed 2× for: the real write
// cost is 1.6× what was shown. The transcript has carried the answer the whole
// time, in `usage.cache_creation`, which tags each write with its TTL.
//
// TWIN: services/claudemon/src/session/usage.rs (turn_cost_usd). The two are
// pinned to each other by contracts/model-pricing-cases.json's
// cacheMultiplierCases block. Edit one side and the other's test goes red.

/** Cache writes held for 5 minutes bill at 1.25× the base input rate. */
export const CACHE_WRITE_5M_MULTIPLIER = 1.25;
/** Cache writes held for 1 hour bill at 2× the base input rate. */
export const CACHE_WRITE_1H_MULTIPLIER = 2;
/** Cache reads bill at 0.1× the base input rate, absent a per-model rate. */
export const CACHE_READ_MULTIPLIER = 0.1;

/**
 * A turn's cache-write cost, weighted by the TTL split.
 *
 * Returned in the same pre-division units as the rest of {@link turnCostUSD}:
 * tokens times a USD-per-million rate, divided by a million once at the end. A
 * turn holding both lifetimes pays each portion at its own rate rather than one
 * blended guess.
 *
 * THE NO-SPLIT FALLBACK, stated rather than assumed: when a turn reports writes
 * but no `cache_creation` block, we cannot tell which lifetime was bought, and
 * we price the whole amount at the 1-hour rate. Defaulting to the cheaper 5m
 * rate is the exact bug this function exists to fix. It reads as a lower bill
 * than the account will see, and a cost readout that is too low is worse than
 * one that is too high. The same rule covers writes the split does not account
 * for (`cache_creation_input_tokens` larger than the two TTL fields sum to).
 */
export function cacheWriteCostUSD(inputRate: number, usage: RawUsage): number {
  const total = usage.cache_creation_input_tokens ?? 0;
  const m5 = usage.cache_creation?.ephemeral_5m_input_tokens;
  const m1h = usage.cache_creation?.ephemeral_1h_input_tokens;
  const rate5m = inputRate * CACHE_WRITE_5M_MULTIPLIER;
  const rate1h = inputRate * CACHE_WRITE_1H_MULTIPLIER;
  if (m5 === undefined && m1h === undefined) return total * rate1h;
  const split = (m5 ?? 0) + (m1h ?? 0);
  const unattributed = Math.max(0, total - split);
  return (m5 ?? 0) * rate5m + (m1h ?? 0) * rate1h + unattributed * rate1h;
}

/** USD cost of a single turn. Cache writes bill per TTL (see
 *  {@link cacheWriteCostUSD}), reads at 0.1× input. */
export function turnCostUSD(model: string | null | undefined, usage: RawUsage): number {
  const r = ratesFor(model);
  // Cache reads default to 0.1× input, but a user override (cached_input) wins —
  // matching the Rust engine's `cached_input.unwrap_or(input * 0.1)`.
  const cacheRead =
    typeof r.cachedInput === 'number' ? r.cachedInput : r.input * CACHE_READ_MULTIPLIER;
  const dollars =
    (usage.input_tokens ?? 0) * r.input +
    cacheWriteCostUSD(r.input, usage) +
    (usage.cache_read_input_tokens ?? 0) * cacheRead +
    (usage.output_tokens ?? 0) * r.output;
  return dollars / 1_000_000;
}

/**
 * The fresh / cache-write / cache-read split of one turn's prompt, or null when
 * the provider reported no cache fields at all.
 *
 * Null is the honest answer for a turn that says nothing about caching: folding
 * it in as three zeros would make a provider that does not itemize look like one
 * that cached nothing, and every surface downstream would draw a 0% hit rate it
 * has no basis for.
 */
export function cacheSplitOf(usage: RawUsage): CacheTokenSplit | null {
  const write = usage.cache_creation_input_tokens;
  const read = usage.cache_read_input_tokens;
  if (write === undefined && read === undefined && usage.cache_creation === undefined) return null;
  return {
    fresh: usage.input_tokens ?? 0,
    write: write ?? 0,
    read: read ?? 0,
  };
}

export function emptyUsage(): SessionUsage {
  return {
    model: null,
    contextTokens: 0,
    // NOT 200_000. This is the empty usage a pane renders before a single
    // token has been counted, and it used to assert a 200k window there — the
    // direct cause of "every session starts at 200k and upgrades later".
    contextLimit: null,
    totalInputTokens: 0,
    totalOutputTokens: 0,
    costUSD: 0,
    models: {},
  };
}
