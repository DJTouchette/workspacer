/**
 * The context window: one table, one resolver.
 *
 * TWINS: `services/claudemon/src/session/windows.rs` (Rust) and
 * `services/hub/cmd/brain/windows.go` (Go). All three are pinned to
 * `contracts/model-context-windows.json` by `modelContextWindowsContract.test.ts`
 * and its opposite numbers — edit one table and the others go red.
 *
 * This replaced five hand-maintained window tables, three of which disagreed
 * for the same model id: a `gpt-5-codex` session reported 272_000 through the
 * provider status line and 200_000 through `usage`, at the same instant, for
 * the same session, depending on which client asked.
 *
 * The other half of what lives here is the shape of NOT KNOWING. A window is
 * `number | null` and `null` means "we do not know", which is a different fact
 * from any number. It used to be spelled `200_000` in four places — including
 * `emptyUsage()`, which asserted a 200k window before a single token had been
 * counted, and which is the whole of the "every session starts at 200k and
 * upgrades later" complaint.
 */

/** How a table row matches a model id (already lowercased).
 *
 *  Normative, not an implementation detail: the `o3`/`o4` rows are `prefix`
 *  rows precisely so an `o3` buried inside an unrelated id does not match, and
 *  a port that read every row as a substring would pass every lookup case in
 *  the fixture except that one. */
export type WindowMatchKind = 'contains' | 'prefix' | 'suffix';

export interface WindowRow {
  /** Lowercased needle. */
  match: string;
  kind: WindowMatchKind;
  window: number;
}

/** THE TABLE, in order: first match wins, so the specific rows come before the
 *  families they overlap. Pinned row-for-row to the fixture's `windows` block. */
export const CONTEXT_WINDOWS: readonly WindowRow[] = [
  // Marker rows first — a statement about the WINDOW outranks a statement
  // about the family, whichever family carries it.
  { match: '[1m]', kind: 'suffix', window: 1_000_000 },
  { match: '-1m', kind: 'suffix', window: 1_000_000 },
  // 1M-native: the max window is also the default, so these ids never carry a
  // marker. Before the generic claude row or their gauges read 5× too full.
  { match: 'fable', kind: 'contains', window: 1_000_000 },
  { match: 'mythos', kind: 'contains', window: 1_000_000 },
  { match: 'gemini', kind: 'contains', window: 1_048_576 },
  { match: 'gpt-4.1', kind: 'contains', window: 1_047_576 },
  // Table KNOWLEDGE, not a fallback: an unmarked Claude model really does hold
  // 200k. The four places that spelled *unknown* 200_000 are gone.
  { match: 'claude', kind: 'contains', window: 200_000 },
  { match: 'gpt-5', kind: 'contains', window: 272_000 },
  { match: 'codex', kind: 'contains', window: 272_000 },
  { match: 'gpt-4o', kind: 'contains', window: 128_000 },
  { match: 'o3', kind: 'prefix', window: 200_000 },
  { match: 'o4', kind: 'prefix', window: 200_000 },
  { match: '/o3', kind: 'contains', window: 200_000 },
  { match: '/o4', kind: 'contains', window: 200_000 },
  { match: 'grok', kind: 'contains', window: 256_000 },
  { match: 'deepseek', kind: 'contains', window: 131_072 },
  { match: 'kimi', kind: 'contains', window: 262_144 },
  { match: 'qwen', kind: 'contains', window: 262_144 },
];

/** How far past the claimed window a session may be observed before we stop
 *  believing the claim. Two percent absorbs the provider's own rounding. */
export const DRIFT_TOLERANCE = 1.02;

export interface ModelSelection {
  /** Provider-native identity only. Legacy context markers never survive here. */
  model: string;
  /** Explicit token window, or null when the caller genuinely did not choose one. */
  contextWindow: number | null;
}

export type ModelSelectionErrorCode =
  | 'empty-model'
  | 'invalid-model-type'
  | 'invalid-context-window'
  | 'conflicting-context-window'
  | 'conflicting-model-identity';

/** A stable error code lets config and wire adapters reject bad mixed-version
 *  input without depending on prose shared across three languages. */
export class ModelSelectionError extends Error {
  constructor(
    readonly code: ModelSelectionErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'ModelSelectionError';
  }
}

const LEGACY_ONE_MILLION_SUFFIX = /(?:\[1m\]|-1m)$/i;

/**
 * Canonicalize a model selection at ingress.
 *
 * Only trailing `[1m]` / `-1m` are syntax. Unknown identities otherwise pass
 * through untouched, and successful output can never carry either suffix.
 */
export function normalizeModelSelection(
  model: string,
  contextWindow?: number | null,
): ModelSelection {
  let identity = model.trim();
  if (!identity) {
    throw new ModelSelectionError('empty-model', 'model selection must name a model');
  }

  let legacyWindow: number | null = null;
  while (LEGACY_ONE_MILLION_SUFFIX.test(identity)) {
    identity = identity.replace(LEGACY_ONE_MILLION_SUFFIX, '').trimEnd();
    legacyWindow = 1_000_000;
  }
  if (!identity) {
    throw new ModelSelectionError(
      'empty-model',
      'model selection cannot consist only of a window marker',
    );
  }

  const explicit = contextWindow ?? null;
  if (explicit !== null && (!Number.isSafeInteger(explicit) || explicit <= 0)) {
    throw new ModelSelectionError(
      'invalid-context-window',
      'contextWindow must be a positive integer token count',
    );
  }
  if (legacyWindow !== null && explicit !== null && legacyWindow !== explicit) {
    throw new ModelSelectionError(
      'conflicting-context-window',
      `legacy model marker selects ${legacyWindow} tokens but contextWindow selects ${explicit}`,
    );
  }

  return { model: identity, contextWindow: explicit ?? legacyWindow };
}

/** Serialize a canonical selection at Claude Code's external argv boundary.
 *  This is the sole current-output marker emitter. It normalizes defensively
 *  so transitional callers that still pass `opus[1m]` remain idempotent. */
export function claudeArgvModel(selection: ModelSelection): string {
  const normalized = normalizeModelSelection(selection.model, selection.contextWindow);
  return normalized.contextWindow === 1_000_000 &&
    !isClaudeInherentOneMillionModel(normalized.model)
    ? `${normalized.model}[1m]`
    : normalized.model;
}

/** Models whose million-token window is inherent, not a selectable Claude
 * variant. They must never receive `[1m]`: unlike Opus/Sonnet, the bare model
 * already selects its only window and Claude Code does not expose a decorated
 * alias for it. Shared argv fixture cases pin this list across TS, Rust and Go. */
export function isClaudeInherentOneMillionModel(model: string): boolean {
  const identity = normalizeModelSelection(model).model.toLowerCase();
  return identity.includes('fable') || identity.includes('mythos');
}

/**
 * Stable identity for a picker row. A bare model id is not enough: Opus and
 * Sonnet each have independently selectable 200K and 1M rows. JSON keeps the
 * pair lossless without turning the UI key itself into provider argv syntax.
 */
export function modelSelectionKey(selection: ModelSelection): string {
  const normalized = normalizeModelSelection(selection.model, selection.contextWindow);
  return JSON.stringify([normalized.model, normalized.contextWindow]);
}

/** Decode a picker key, accepting an old model-string value during rollout. */
export function modelSelectionFromKey(key: string): ModelSelection {
  try {
    const parsed = JSON.parse(key) as unknown;
    if (
      Array.isArray(parsed) &&
      parsed.length === 2 &&
      typeof parsed[0] === 'string' &&
      (parsed[1] === null || typeof parsed[1] === 'number')
    ) {
      return normalizeModelSelection(parsed[0], parsed[1]);
    }
  } catch {
    // Legacy controlled-select values were model strings, including `[1m]`.
  }
  return normalizeModelSelection(key);
}

export function sameModelSelection(a: ModelSelection, b: ModelSelection): boolean {
  const left = normalizeModelSelection(a.model, a.contextWindow);
  const right = normalizeModelSelection(b.model, b.contextWindow);
  return (
    left.model.toLowerCase() === right.model.toLowerCase() &&
    left.contextWindow === right.contextWindow
  );
}

/**
 * The table's answer for a concrete model id, or `null` when no row covers it.
 *
 * `null` is the honest unknown and it is load-bearing: every readout in the
 * repo already hides its meter on an absent window, so a model this table has
 * never heard of shows no meter instead of a familiar-looking wrong one.
 */
export function windowFor(model: string | null | undefined): number | null {
  if (!model) return null;
  const m = model.toLowerCase();
  for (const row of CONTEXT_WINDOWS) {
    const hit =
      row.kind === 'prefix'
        ? m.startsWith(row.match)
        : row.kind === 'suffix'
          ? m.endsWith(row.match)
          : m.includes(row.match);
    if (hit) return row.window;
  }
  return null;
}

/**
 * The window implied by the model string a session was *asked* for — the alias
 * the user picked (`opus[1m]`), not the concrete id the transcript records.
 *
 * Deliberately narrower than `windowFor`: it answers only "was 1M asked for",
 * and `null` means "says nothing", NOT "200k". A bare `opus` may be a 200k
 * session or whatever Claude Code's default becomes tomorrow; pinning a number
 * here is exactly how a wrong window gets asserted from token zero.
 */
export function requestedWindowFor(model: string | null | undefined): number | null {
  if (!model) return null;
  let selection: ModelSelection;
  try {
    selection = normalizeModelSelection(model);
  } catch {
    return null;
  }
  const m = selection.model.toLowerCase();
  if (selection.contextWindow === 1_000_000 || m.includes('fable') || m.includes('mythos')) {
    return 1_000_000;
  }
  return null;
}

/** Everything beyond the transcript's `model` id that can speak to a session's
 *  window. Each is optional; a session with none of them falls back to the
 *  table, and then to honest unknown. */
export interface WindowSignals {
  /** The window the provider itself reported, in tokens. For Claude this is
   *  `context_window.context_window_size` off the statusLine payload (PTY) or
   *  the stream `result` frame's `modelUsage.*.contextWindow`; for Codex it is
   *  `model_context_window`. Real provider truth, so it outranks every guess. */
  reportedWindow?: number | null;
  /** The user's `~/.workspacer/model-rates.json` `context_limit`. */
  overrideWindow?: number | null;
  /** The model string this session was *asked* for at spawn — the alias the
   *  user picked (`opus[1m]`), not the concrete id. Claude Code strips the
   *  `[1m]` marker from the transcript's `model` field, so this is the only
   *  place the 1M choice survives before the provider reports a window. */
  requestedModel?: string | null;
  /** The session's high-water context occupancy, for the drift alarm. */
  peakContext?: number;
}

/**
 * THE RESOLVER. One hierarchy, and the order is the design:
 *
 *   1. `reportedWindow` — the window the provider gave for THIS session. A fact.
 *   2. `overrideWindow` — the user's `model-rates.json` `context_limit`. They
 *      are overruling us deliberately, so it outranks the marker below. This
 *      REPLACED a `Math.max()`, under which a coarse alias could silently raise
 *      the window back over what the user wrote.
 *   3. `requestedModel`'s `[1m]` marker — known from token zero, which is what
 *      makes birth-time knowledge possible.
 *   4. the contract table, by concrete model id.
 *   5. `null` — unknown. Never 200_000.
 *
 * Then the DRIFT ALARM. A claim `peakContext` EXCEEDS (past `DRIFT_TOLERANCE`)
 * is demonstrably wrong, so that claim is DISQUALIFIED and the next one down
 * the hierarchy is tried; unknown is the answer only once every claim has been
 * disproved. This is the retrospective 200k→1M promotion, demoted from a source
 * of truth to an alarm: it used to silently REWRITE the window to 1M, a guess
 * dressed as a correction.
 *
 * Disqualify-and-CONTINUE, not disqualify-and-stop. Stopping discarded claims
 * the evidence had never touched, and that cost a real reading: Claude Code's
 * own statusLine reports `context_window_size: 200000` for a session spawned
 * `opus[1m]`, so a live 1M worker holding 356k tokens had its REPORTED window
 * disproved and then resolved to UNKNOWN — instead of falling through to the
 * `[1m]` marker, which says 1M and which 356k does not contradict. An unknown
 * window is what made that worker's context bar peg at 100%. Falling through
 * invents nothing: every candidate below already existed and was already
 * ranked; the alarm now only removes the ones the session disproved.
 */
export function resolveContextWindow(
  model: string | null | undefined,
  signals?: WindowSignals,
): number | null {
  const peak = signals?.peakContext ?? 0;
  for (const claim of windowClaims(model, signals)) {
    if (peak > claim * DRIFT_TOLERANCE) {
      console.warn(
        `[contextWindow] drift: session on model ${JSON.stringify(model)} ` +
          `(requested ${JSON.stringify(signals?.requestedModel ?? null)}) holds ${peak} tokens, ` +
          `past the ${claim}-token window claimed for it. Disqualifying that claim and trying ` +
          `the next. If every claim is disproved the window is UNKNOWN, and if this model is ` +
          `real, contracts/model-context-windows.json is stale for it.`,
      );
      continue;
    }
    return claim;
  }
  return null;
}

/** The hierarchy, in order, as a LIST rather than a single answer — so the
 *  alarm can drop a disproved claim and keep going. Split out so the alarm has
 *  something to compare against and so tests can name the two halves separately. */
function windowClaims(model: string | null | undefined, signals?: WindowSignals): number[] {
  const out: number[] = [];
  const reported = signals?.reportedWindow;
  if (typeof reported === 'number' && Number.isFinite(reported) && reported > 0) out.push(reported);
  const override = signals?.overrideWindow;
  if (typeof override === 'number' && Number.isFinite(override) && override > 0) out.push(override);
  const requested = requestedWindowFor(signals?.requestedModel);
  if (requested !== null) out.push(requested);
  const table = windowFor(model);
  if (table !== null) out.push(table);
  return out;
}

/**
 * The model picker's `context` badge for one of Claude Code's own aliases.
 *
 * The alias rows in `claudeModels.ts` (and their Go twin in
 * `cmd/brain/models.go`) used to hardcode `'200K'` / `'1M'`, which is how a
 * display-only sixth window table came to exist alongside the four numeric ones.
 *
 * DOMAIN: a `claude.listModels` alias — `opus`, `sonnet[1m]`, `haiku`, `fable` —
 * or a concrete Claude id the user has been seen running. Nothing else is ever
 * passed here. That matters because a bare alias names no vendor and so matches
 * no row: the table keys the family on the string `claude`, which a concrete
 * transcript id always carries and an alias never does. Hence the second
 * lookup — within this domain, prepending the family is the identity, not a
 * guess. A marker alias (`opus[1m]`) or a 1M-native one (`fable`) is answered by
 * the first lookup and never reaches it.
 *
 * TWIN: `formatClaudeAliasWindow` in services/hub/cmd/brain/windows.go. The two
 * answers are pinned to each other by contracts/claude-model-catalog-cases.json.
 */
export function formatClaudeAliasWindow(alias: string): string {
  const w = claudeAliasSelection(alias).contextWindow;
  // An id the table has never heard of gets no badge rather than an invented
  // one — the same honest unknown as everywhere else.
  return formatContextWindow(w);
}

/**
 * Resolve one Claude catalog alias through the shared window contract. The
 * catalog may describe the 1M variants with legacy syntax, but its result is a
 * canonical suffix-free identity plus an explicit window.
 */
export function claudeAliasSelection(alias: string): ModelSelection {
  const normalized = normalizeModelSelection(alias);
  return {
    model: normalized.model,
    contextWindow:
      normalized.contextWindow ??
      windowFor(normalized.model) ??
      windowFor(`claude-${normalized.model}`),
  };
}

/** Compact badge for an already-selected numeric window. */
export function formatContextWindow(window: number | null | undefined): string {
  if (window == null) return '';
  if (window >= 1_000_000) return '1M';
  return `${Math.round(window / 1000)}K`;
}
