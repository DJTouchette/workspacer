/**
 * Compose a brief line from a worker's STRUCTURED RESULT instead of retyping it.
 *
 * WHY THIS EXISTS. A `Recently` line is two different things welded together: a
 * sentence of JUDGEMENT ("this unblocks the mobile client") that only the
 * manager can write, and a run of MECHANICAL FACTS (the commit, the files, the
 * checks, the caveats, the session it came from) that the worker already
 * reported verbatim in its `wks-result` block. Today the manager retypes both.
 * That costs tokens on the half a machine can render, and — the reason this
 * landed — it MISTRANSCRIBES. A live manager wrote `session:6a-round2` into a
 * brief: a nickname where a session id belongs, produced by a model composing a
 * reference from memory rather than copying one. The brief's `session:<id>`
 * links stopped resolving and the line had to be repaired by hand, which an
 * additive-only tool cannot do for you.
 *
 * So the split is: THE MANAGER WRITES THE SENTENCE, THE HOST WRITES THE FACTS.
 *
 * THE HARD RULE, and it is dispatchTemplate.ts's rule wearing different clothes:
 * the significance sentence is REQUIRED and a result object alone can never
 * produce a line. A dispatch template refuses to render without its `{{task}}`
 * slot for the same reason — the rendered thing READS FINISHED, so a caller who
 * could skip the judgement slot would ship boilerplate that looks like
 * reasoning. A brief line assembled purely from `{commit, filesChanged}` reads
 * like a considered entry and says nothing about why anyone should care.
 * Refusing is louder and cheaper than a brief full of machine exhaust.
 *
 * THE SECOND HARD RULE: a malformed session id is REFUSED, never guessed at,
 * never passed through. That is the entire error class this feature exists to
 * kill, and passing `6a-round2` through would reproduce the bug through a
 * fancier door.
 *
 * LOSSY BUT HONEST. Long arrays are capped and say so (`+4 more`); they are
 * never quietly shortened. CAVEATS ARE NEVER CAPPED AT ALL — a caveat is the
 * one field whose whole value is that somebody reads it, and a brief line that
 * silently drops "the migration is not reversible" is worse than no line. If
 * the caveats make the line too long, briefService REFUSES the whole write,
 * which is the correct loud failure.
 *
 * TWIN: services/hub/cmd/brain/briefresult.go.
 */
import { hasNonBlankText } from './asciiWhitespace';

/** How many items of a capped array are shown before the `+K more` tail. */
export const FACT_ITEMS_SHOWN = 3;

/** Longest a single rendered fact VALUE may be before it is cut with an
 *  explicit ellipsis. Caveats are exempt (see the module header). */
export const FACT_VALUE_MAX = 200;

/**
 * The fields of the common `wks-result` payload, in the order a reader wants
 * them: what landed, where, what proved it, what is still wrong, what is next.
 * Any OTHER key an arbitrary result carries is rendered after these, in its own
 * insertion order — the payload is caller-defined JSON and this must not become
 * an allowlist that silently drops a field somebody chose to report.
 */
export const FACT_ORDER = ['commit', 'filesChanged', 'checksRun', 'caveats', 'followUps'];

/** Keys rendered WITHOUT any cap. See the module header: a dropped caveat is
 *  the one loss this renderer refuses to take. */
const UNCAPPED_KEYS = new Set(['caveats', 'caveat']);

/** A session reference as it is written into a brief: `session:` plus a lowercase
 *  hex run. Kept in step with briefBoard.ts's REF_RE (`session:[0-9a-f]{6,}`),
 *  which is what makes the reference a CLICKABLE card ref rather than prose. */
export const SESSION_REF_SHORT_LEN = 8;

const BARE_HEX_RE = /^[0-9a-f]{6,}$/;
const UUID_RE = /^([0-9a-f]{8})-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export class MalformedSessionRef extends Error {
  constructor(readonly given: string) {
    super(
      `brief.append: ${JSON.stringify(given)} is not a session id, so nothing was written. ` +
        "A session reference is the worker's own id — a full UUID, or at least its first 6 " +
        'hex characters — and it is written into the brief as `session:<id>` so the user can ' +
        'click through to that agent. A label, a round number, a nickname or a slug ' +
        '("6a-round2") is not a session id and would leave a dead link in the user\'s brief. ' +
        'Copy the id from list_agents or from the wake that reported the result.',
    );
    this.name = 'MalformedSessionRef';
  }
}

/**
 * Validate a caller-supplied session id and return the CANONICAL SHORT FORM the
 * brief's `session:<id>` convention uses.
 *
 * Tolerated on the way in: surrounding whitespace, a `session:` prefix the
 * caller typed out of habit, and upper-case hex. Refused: anything that is not
 * a hex run of 6+ or a full UUID. Refusal is the point — see MalformedSessionRef.
 *
 * The short form is the first 8 hex characters (a UUID's first group), which is
 * what FleetMessageCard already shortens a live session to and what the briefs
 * in this repo already contain.
 */
export function normalizeSessionRef(raw: unknown): string {
  const text = String(raw ?? '').trim();
  const bare = text
    .replace(/^session:/i, '')
    .trim()
    .toLowerCase();
  if (!bare) throw new MalformedSessionRef(text);
  const uuid = UUID_RE.exec(bare);
  if (uuid) return uuid[1];
  if (!BARE_HEX_RE.test(bare)) throw new MalformedSessionRef(text);
  return bare.slice(0, SESSION_REF_SHORT_LEN);
}

/** True when `raw` is a session id this module would accept. Used by the stale
 *  check to tell a BROKEN reference from a merely dead one. */
export function isSessionRef(raw: unknown): boolean {
  try {
    normalizeSessionRef(raw);
    return true;
  } catch {
    return false;
  }
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** Flatten a value to one line of text. Objects and arrays-of-objects become
 *  compact JSON rather than `[object Object]`. */
function scalarText(v: unknown): string {
  if (typeof v === 'string') return v.replace(/\s+/g, ' ').trim();
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  if (v === null) return 'null';
  try {
    return JSON.stringify(v) ?? '';
  } catch {
    return '';
  }
}

function cut(text: string, uncapped: boolean): string {
  if (uncapped || text.length <= FACT_VALUE_MAX) return text;
  // Announced, never silent — the same principle briefService's refusal rests
  // on, one severity down because this is a field and not the whole line.
  return `${text.slice(0, FACT_VALUE_MAX)}… (${text.length} chars)`;
}

/** A commit sha is rendered short, the way every other tool in this repo shows
 *  one. Only a real 40-hex sha is shortened: an abbreviated sha, a tag or a
 *  sentence is left exactly as the worker reported it. */
function renderCommit(text: string): string {
  return /^[0-9a-f]{40}$/i.test(text) ? text.slice(0, 12) : text;
}

/** Render ONE key/value pair, or null when the value carries no fact at all. */
function renderFact(key: string, value: unknown): string | null {
  const uncapped = UNCAPPED_KEYS.has(key.toLowerCase());
  if (value === undefined || value === null) return null;
  if (Array.isArray(value)) {
    const items = value.map(scalarText).filter((s) => s !== '');
    if (items.length === 0) return null;
    const shown = uncapped ? items : items.slice(0, FACT_ITEMS_SHOWN);
    const hidden = items.length - shown.length;
    // "+K more" rather than a bare truncation: the reader is told a number is
    // missing and how big it is, which is the difference between lossy and
    // dishonest.
    const body = hidden > 0 ? `${shown.join(', ')}, +${hidden} more` : shown.join(', ');
    return `${key}: ${cut(body, uncapped)}`;
  }
  const text = scalarText(value);
  if (text === '') return null;
  if (key.toLowerCase() === 'commit') return `${key}: ${renderCommit(text)}`;
  return `${key}: ${cut(text, uncapped)}`;
}

/**
 * Render a worker's parsed result object as ONE compact run of facts.
 *
 * `result` is treated as ARBITRARY JSON, because it is: the schema is written
 * per dispatch by the manager, and `{commit, filesChanged, checksRun, caveats,
 * followUps}` is only the common shape. Known keys lead, in FACT_ORDER;
 * everything else follows in its own order. Empty values (null, "", [], {})
 * carry no fact and are dropped — reporting `caveats: ` would be noise, and
 * an EMPTY caveats list is a truthful "none", not a caveat.
 */
export function renderResultFacts(result: unknown): string {
  if (result === undefined || result === null) return '';
  if (!isPlainObject(result)) {
    const one = renderFact('result', result);
    return one ?? '';
  }
  const keys = Object.keys(result);
  const ordered = [
    ...FACT_ORDER.filter((k) => keys.includes(k)),
    ...keys.filter((k) => !FACT_ORDER.includes(k)),
  ];
  const parts: string[] = [];
  for (const key of ordered) {
    const rendered = renderFact(key, result[key]);
    if (rendered !== null) parts.push(rendered);
  }
  return parts.join('; ');
}

/** `YYYY-MM-DD` in LOCAL time — a brief is a human's dated log, so "today"
 *  means the user's today, not UTC's. Matches the doctrine's `- YYYY-MM-DD  …`
 *  format, double space included (see normalizeBriefLine's note on why that
 *  second space survives normalization). */
export function isoDay(now: Date = new Date()): string {
  const p = (n: number): string => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

const LEADING_DATE_RE = /^\d{4}-\d{2}-\d{2}\b/;

/** Flatten the manager's sentence to one line — briefService.normalizeBriefLine's
 *  exact two replaces, and INTERIOR SPACES ARE LEFT ALONE for its exact reason:
 *  a `\s+ → ' '` collapse also eats the double space in the doctrine's own dated
 *  format (`- YYYY-MM-DD  <what happened>`), so a manager backfilling a dated
 *  line would have had its date separator quietly re-spaced by the one tool that
 *  exists to write those lines. */
function flattenSentence(text: string): string {
  return text
    .replace(/[ \t\f\v]*[\r\n]+[ \t\f\v]*/g, ' ')
    .replace(/[\t\f\v]+/g, ' ')
    .trim();
}

export interface ComposeResultLineInput {
  /** The manager's own one-sentence significance line. REQUIRED. */
  significance: string;
  /** The worker's session id, in any accepted spelling. */
  sessionId?: unknown;
  /** The worker's parsed `wks-result` payload. Arbitrary JSON. */
  result?: unknown;
  /** Injectable clock, for tests. */
  now?: Date;
}

/** True when a call carries the from-result params at all. When this is false,
 *  `brief_append` must behave EXACTLY as it always has. */
export function hasResultParams(input: { sessionId?: unknown; result?: unknown }): boolean {
  return input.sessionId !== undefined || input.result !== undefined;
}

/**
 * Compose the final brief line: date, the manager's sentence, the mechanical
 * facts, the session reference.
 *
 * Throws when the significance sentence is missing or blank (the module
 * header's first hard rule) and when the session id is malformed (the second).
 * Everything it produces still goes through briefService's own normalization
 * and length refusal — this composes a candidate line, it does not write one.
 */
export function composeResultLine(input: ComposeResultLineInput): string {
  const significance = flattenSentence(String(input.significance ?? ''));
  if (!hasNonBlankText(significance)) {
    // HARD ERROR, by design — see the module header, and dispatchTemplate.ts's
    // identical refusal for the identical reason.
    throw new Error(
      'brief.append: `line` must carry your own one-sentence significance line — ' +
        'what this result MEANS for the project — and nothing was written without it. ' +
        'The result object supplies the mechanical facts (commit, files, checks, ' +
        'caveats) and the host renders them for you; the judgement is the part only ' +
        'you can write, so a result on its own can never become a brief line.',
    );
  }

  const ref = input.sessionId === undefined ? '' : normalizeSessionRef(input.sessionId);
  const facts = renderResultFacts(input.result);

  // A sentence the manager already dated keeps ITS date: re-prefixing would
  // produce `- 2026-08-26  2026-08-26 …`, and the caller's date may be
  // deliberate (backfilling yesterday's entry).
  const dated = LEADING_DATE_RE.test(significance)
    ? significance
    : `${isoDay(input.now)}  ${significance}`;

  // Likewise, a sentence that already names this session is not given a second
  // copy of the same reference.
  const alreadyRefs = ref !== '' && new RegExp(`session:${ref}`, 'i').test(dated);

  return [dated, facts ? ` — ${facts}` : '', ref && !alreadyRefs ? ` (session:${ref})` : ''].join(
    '',
  );
}
