/**
 * The canonical model-selection slice a session snapshot carries — the wire
 * MAPPING for it, and nothing else.
 *
 * ADDITIVE. Two optional fields ride alongside everything a snapshot already
 * had, and nothing existing changes shape:
 *
 *   - `requestedSelection` — the canonical `{model, contextWindow}` pair this
 *     session was ASKED for, suffix-free. The compatibility projection older
 *     readers know is still `settings.model` (which may spell the 1M variant
 *     `opus[1m]`); this is the same fact without the argv syntax.
 *   - `resolvedContextWindow` — the window the OWNER settled on for this
 *     session, in tokens. ABSENT means nobody has said, which is a different
 *     fact from any number and is why the field is optional, never defaulted.
 *
 * CLAUDEMON OWNS BOTH FIELDS. The desktop is a receiver and a forwarder, never
 * a producer: it maps the daemon's spelling, presence-merges the result into
 * session state, and passes it on. There is deliberately NO local
 * reconstruction from the legacy fields — deriving `requestedSelection` from
 * `settings.model`, or `resolvedContextWindow` from `contextLimitFor`, would
 * recreate the second, disagreeing answer this slice exists to retire.
 *
 * A receiver therefore does not re-pair these, does not re-derive them, and
 * above all does not discard them on occupancy: the drift check
 * (`DRIFT_TOLERANCE`) belongs to the RAW provider status pair
 * (`statusLine.contextWindowSize` beside a measured token count) and stays
 * there — see `busContextLimit` in hubCapabilities.ts, which is the one place a
 * disproved raw window falls through to this canonical field as the DISPLAY
 * denominator. What is stored never changes because of what is displayed.
 *
 * WHY A MODULE AND NOT A SPREAD: claudemon owns these in snake_case
 * (`requested_selection: {model, context_window}`, `resolved_context_window` —
 * services/claudemon session/windows.rs). The combined brain now emits the
 * camelCase pair ALONGSIDE those originals, so a CURRENT peer row carries both.
 * Every reader here still accepts either, deliberately: version skew is normal
 * across a federation link and across an adopted hub, and an older brain sends
 * only the snake form — the same way `status_line` and `tool_calls` still ride
 * along unrenamed today. Reading both costs one `??` and removes a whole class
 * of "that peer's fleet has no window" report.
 */

import type { ModelSelection } from './modelContextWindows';

/** The additive pair, as it rides on a snapshot. Both optional; absent means
 *  "nobody has said", never a default. */
export interface CanonicalSelectionSlice {
  /** Canonical, suffix-free request. `contextWindow: null` = no window chosen. */
  requestedSelection?: ModelSelection;
  /** The owner's resolved window in tokens. Absent = unknown; never a guessed
   *  200_000. Stored exactly as received. */
  resolvedContextWindow?: number;
}

/** A positive integer token count, or `undefined` for anything else — a float,
 *  a zero, a string, a null. Zero is not a window; some producers spell
 *  "unknown" that way. */
function tokenCount(raw: unknown): number | undefined {
  return typeof raw === 'number' && Number.isSafeInteger(raw) && raw > 0 ? raw : undefined;
}

function asRecord(raw: unknown): Record<string, unknown> | null {
  return raw && typeof raw === 'object' && !Array.isArray(raw)
    ? (raw as Record<string, unknown>)
    : null;
}

/**
 * Read the owner's canonical request off a wire row, in either spelling:
 * `requestedSelection: {model, contextWindow}` (a peer desktop's own snapshot)
 * or `requested_selection: {model, context_window}` (claudemon's original,
 * which a headless peer forwards unrenamed).
 *
 * Carried across VERBATIM — the model string is not re-normalized and the
 * window is not re-derived from it. The only rejection is a row that does not
 * carry a usable pair at all (no model string, or a window that is not a
 * positive integer), which reports absence rather than a half-read selection.
 */
export function readRequestedSelection(row: unknown): ModelSelection | undefined {
  const raw = asRecord(asRecord(row)?.requestedSelection ?? asRecord(row)?.requested_selection);
  if (!raw) return undefined;
  const model = raw.model;
  if (typeof model !== 'string' || !model) return undefined;
  const windowRaw = raw.context_window !== undefined ? raw.context_window : raw.contextWindow;
  // `null` is a real answer here ("no window chosen") and rides through. A
  // present-but-unusable window is not silently downgraded to null: the pair is
  // dropped, because half a selection is not the owner's fact.
  if (windowRaw === null || windowRaw === undefined) return { model, contextWindow: null };
  const contextWindow = tokenCount(windowRaw);
  return contextWindow === undefined ? undefined : { model, contextWindow };
}

/** Read the owner's resolved window off a wire row, in either spelling. */
export function readResolvedContextWindow(row: unknown): number | undefined {
  const r = asRecord(row);
  if (!r) return undefined;
  return tokenCount(
    r.resolvedContextWindow !== undefined ? r.resolvedContextWindow : r.resolved_context_window,
  );
}

/** Both fields off one wire row. A key the row does not carry stays absent —
 *  that absence is what `mergeSelectionSlice` reads as "this update said
 *  nothing about it". */
export function readSelectionSlice(row: unknown): CanonicalSelectionSlice {
  const out: CanonicalSelectionSlice = {};
  const requestedSelection = readRequestedSelection(row);
  if (requestedSelection) out.requestedSelection = requestedSelection;
  const resolvedContextWindow = readResolvedContextWindow(row);
  if (resolvedContextWindow !== undefined) out.resolvedContextWindow = resolvedContextWindow;
  return out;
}

/**
 * PRESENCE-AWARE merge, and that is ALL it is: `overlay` wins each field it
 * actually carries, `base` keeps the rest. Nothing is validated, re-paired or
 * dropped here.
 *
 * The rule exists because a sparse update — a headless peer's state-only row, a
 * layout ghost, a brain `compatSnapshot` — omits most of what a rich row
 * carries. A plain `{...base, ...overlay}` is only accidentally safe: it
 * survives an ABSENT key and silently erases the field on a key present with
 * `undefined`, which is what a hand-built or mapped-then-spread row produces.
 * Reading presence explicitly means a sparse update can ADD an owner fact and
 * can never SUBTRACT one.
 */
export function mergeSelectionSlice(
  base: CanonicalSelectionSlice | undefined,
  overlay: CanonicalSelectionSlice | undefined,
): CanonicalSelectionSlice {
  const out: CanonicalSelectionSlice = {};
  const requestedSelection = overlay?.requestedSelection ?? base?.requestedSelection;
  if (requestedSelection) out.requestedSelection = requestedSelection;
  const resolvedContextWindow = overlay?.resolvedContextWindow ?? base?.resolvedContextWindow;
  if (resolvedContextWindow !== undefined) out.resolvedContextWindow = resolvedContextWindow;
  return out;
}

/** Write a merged slice onto a row: a field nobody knows is DELETED rather than
 *  set to `undefined`, so the stored row keeps the same omit-what-is-unknown
 *  shape the wire has (`JSON.stringify` and the structured clone agree). */
export function applySelectionSlice(
  target: CanonicalSelectionSlice,
  slice: CanonicalSelectionSlice,
): void {
  if (slice.requestedSelection) target.requestedSelection = slice.requestedSelection;
  else delete target.requestedSelection;
  if (slice.resolvedContextWindow !== undefined) {
    target.resolvedContextWindow = slice.resolvedContextWindow;
  } else delete target.resolvedContextWindow;
}
