/**
 * Reading a worker's structured result for display — the logic half of
 * StructuredResultCard, kept separate so it can be tested without a DOM.
 *
 * ## The constraint this module exists for
 *
 * The schema is ARBITRARY. It is authored per dispatch by whichever manager
 * spawned the worker (`spawn_agent`'s `resultSchema`), so the card cannot know
 * the keys ahead of time: today's dispatches have asked for `decisionTaken`,
 * `testsFixed`, `realBugsFound`, `itemsSkipped`, `bytesAdded`, `secretsCheck`.
 * So the card renders by VALUE SHAPE, not by key: a boolean is a badge, a
 * number is a counter chip, an array of paths is a file list, whatever the key
 * is called. A handful of conventional keys (see KNOWN_ORDER) get a better
 * slot and a nicer treatment on top of that, never instead of it.
 *
 * The rule that follows: **no field is ever dropped.** A key nobody
 * anticipated is worse silently missing than shown plainly — the whole point
 * of the contract is that the worker's own report reaches the manager intact.
 *
 * Nothing here throws. A result that is not parseable JSON (the wake caps the
 * object at RESULT_MAX and appends a `[truncated: …]` marker, which makes it
 * invalid on purpose), or is JSON but not an object, comes back as a
 * `fallback` the card shows as raw text with the reason — never an empty card.
 */

/** How a value is rendered. Chosen from the value's shape, not its key. */
export type ResultFieldKind =
  /** true/false — a yes/no badge. */
  | 'boolean'
  /** A finite number — a counter chip. */
  | 'number'
  /** A git SHA — mono, copyable. */
  | 'commit'
  /** Any other non-empty string — a paragraph, clamped when long. */
  | 'text'
  /** An array of strings that look like file paths — FileLinks. */
  | 'paths'
  /** An array of scalars — a bulleted list, collapsed when long. */
  | 'strings'
  /** An array holding objects/arrays — JSON per item. */
  | 'list'
  /** A non-empty object — nested key/value rows. */
  | 'object'
  /** null, "", [], {} — reported, but with nothing in it. */
  | 'empty';

/** Where the card puts a field. `summary` is the scannable strip at the top,
 *  `caveats` is the never-folded band under it, `body` is everything else. */
export type ResultFieldSlot = 'summary' | 'caveats' | 'body';

export interface ResultField {
  /** The key exactly as the worker wrote it. */
  key: string;
  /** The key as words, for the label. */
  label: string;
  kind: ResultFieldKind;
  value: unknown;
  slot: ResultFieldSlot;
}

export interface ResultView {
  /** Every field of the result, in display order. Never lossy. */
  fields: ResultField[];
  /** Set when the payload could not be read as a JSON object; the card shows
   *  `text` verbatim and says `reason`. */
  fallback?: { text: string; reason: string };
}

/**
 * The conventional keys, in the order they read best. Everything else keeps
 * the worker's own key order, after these. `caveats` sits high in this list
 * but takes its own slot — it is the field a manager most needs to see and the
 * one a worker is most tempted to bury.
 */
export const KNOWN_ORDER = [
  'merged',
  'commit',
  'caveats',
  'filesChanged',
  'checksRun',
  'followUps',
] as const;

/** Keys that mean "what the worker could not verify". Never folded away. */
const CAVEAT_KEYS = new Set(['caveat', 'caveats']);

/** Items beyond this in an array are collapsed behind a "+N more" toggle. */
export const ARRAY_PREVIEW = 3;

/** Arrays no longer than this are shown whole — collapsing four items to
 *  three, to save one line, is worse than showing them. */
export const ARRAY_COLLAPSE_MIN = 5;

/** Strings longer than this get a "more" toggle. */
export const TEXT_CLAMP = 280;

/** Caveats get a much longer leash: the field must be READABLE without a
 *  click, so only a pathological one is ever clamped, and its head still
 *  shows. */
export const CAVEATS_CLAMP = 700;

/** An abbreviated (7-char) or full git SHA. */
const SHA_RE = /^[0-9a-f]{7,40}$/i;

/** Key names whose value is a SHA even when the string is unusual. */
const COMMIT_KEY_RE = /(^|[^a-z])(commit|sha|revision|rev)s?$/i;

/** A string that reads like a file path: has a separator, no whitespace, and
 *  is not a URL. Deliberately shape-based — `filesChanged` is the conventional
 *  key, but `filesTouched` / `pathsSkipped` in a future schema get the same
 *  treatment for free. */
const PATH_RE = /^[^\s:]*[/\\][^\s]*$/;

/** camelCase / snake_case / kebab-case → lowercase words. */
export function humanizeKey(key: string): string {
  const words = key
    .replace(/[_-]+/g, ' ')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .trim();
  return words ? words.toLowerCase() : key;
}

/** Whether a value should be shown as a git SHA. */
export function looksLikeCommit(key: string, value: unknown): boolean {
  if (typeof value !== 'string') return false;
  const v = value.trim();
  if (!v) return false;
  if (COMMIT_KEY_RE.test(key)) return v.length <= 64 && !/\s/.test(v);
  return SHA_RE.test(v);
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

const isScalar = (v: unknown): boolean =>
  typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean';

/** The rendering shape of one value. Total: every JSON value lands somewhere. */
export function classifyValue(key: string, value: unknown): ResultFieldKind {
  if (value === null || value === undefined) return 'empty';
  if (typeof value === 'boolean') return 'boolean';
  if (typeof value === 'number') return Number.isFinite(value) ? 'number' : 'empty';
  if (typeof value === 'string') {
    if (!value.trim()) return 'empty';
    return looksLikeCommit(key, value) ? 'commit' : 'text';
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return 'empty';
    if (!value.every(isScalar)) return 'list';
    const strings = value.filter((v): v is string => typeof v === 'string');
    if (strings.length === value.length && strings.every((s) => PATH_RE.test(s))) return 'paths';
    return 'strings';
  }
  if (isPlainObject(value)) return Object.keys(value).length === 0 ? 'empty' : 'object';
  return 'empty';
}

/** Which band of the card a field belongs in. */
function slotFor(key: string, kind: ResultFieldKind): ResultFieldSlot {
  if (CAVEAT_KEYS.has(key.toLowerCase())) return 'caveats';
  // Shape, not name: any boolean is a yes/no answer and any number is a count,
  // and both scan in a glance — so both belong in the top strip whatever an
  // unanticipated schema called them.
  if (kind === 'boolean' || kind === 'number' || kind === 'commit') return 'summary';
  return 'body';
}

/** One field, described for rendering. */
export function describeField(key: string, value: unknown): ResultField {
  const kind = classifyValue(key, value);
  return { key, label: humanizeKey(key), kind, value, slot: slotFor(key, kind) };
}

/** Known keys first (in KNOWN_ORDER), then the worker's own key order. */
function orderKeys(keys: string[]): string[] {
  const rank = (k: string): number => {
    const i = (KNOWN_ORDER as readonly string[]).indexOf(k);
    return i < 0 ? KNOWN_ORDER.length : i;
  };
  return keys
    .map((k, i) => ({ k, i }))
    .sort((a, b) => rank(a.k) - rank(b.k) || a.i - b.i)
    .map((x) => x.k);
}

/**
 * Read a wake's `result` payload into fields to render.
 *
 * Every failure is a `fallback`, never an exception and never an empty card:
 * a truncated object still shows its bytes, and a schema that produced an
 * array or a bare string still shows the value.
 */
export function buildResultView(json: string | undefined | null): ResultView {
  const text = (json ?? '').trim();
  if (!text) {
    return { fields: [], fallback: { text: '', reason: 'the result block was empty' } };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    // The wake truncates an oversized result mid-object on purpose, so this is
    // an expected shape, not a corruption: show the bytes that did arrive.
    const truncated = /\[truncated:/.test(text);
    return {
      fields: [],
      fallback: {
        text,
        reason: truncated
          ? 'the result was too large for the wake and arrived truncated'
          : 'the result is not valid JSON',
      },
    };
  }
  if (!isPlainObject(parsed)) {
    return {
      fields: [describeField('result', parsed)],
      fallback: undefined,
    };
  }
  const fields = orderKeys(Object.keys(parsed)).map((k) => describeField(k, parsed[k]));
  return { fields };
}

/** Fields for one band, in display order. */
export function fieldsInSlot(view: ResultView, slot: ResultFieldSlot): ResultField[] {
  return view.fields.filter((f) => f.slot === slot);
}

/** Short form of a SHA for a chip; anything that isn't a long hex string is
 *  shown whole (a tag or a branch name means nothing abbreviated). */
export function shortCommit(value: string): string {
  const v = value.trim();
  return SHA_RE.test(v) && v.length > 12 ? v.slice(0, 8) : v;
}

/** A number as a counter chip reads better grouped. */
export function formatNumber(value: number): string {
  return Number.isInteger(value) ? value.toLocaleString('en-US') : String(value);
}

/** One list item's display text, whatever the item is. */
export function itemText(item: unknown): string {
  if (typeof item === 'string') return item;
  if (typeof item === 'number' || typeof item === 'boolean') return String(item);
  if (item === null || item === undefined) return '—';
  try {
    return JSON.stringify(item);
  } catch {
    return String(item);
  }
}

/** What an `empty` field says, so "reported as nothing" never reads as "not
 *  reported at all". */
export function emptyLabel(value: unknown): string {
  if (Array.isArray(value)) return 'none';
  if (value === null || value === undefined) return 'not reported';
  if (typeof value === 'string') return 'empty';
  if (typeof value === 'number') return 'not a number';
  return 'none';
}
