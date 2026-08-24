/**
 * The BoardPane's data model: a `.workspacer/brief.md` seen as a set of CARDS
 * that can be moved between the brief's own sections and out into
 * `brief.archive.md`.
 *
 * Pure — no `fs`, no Electron. Shared by main (which does the writing, see
 * services/briefBoardService) and the renderer (which draws the board), because
 * the card shape is a contract between the two and a second copy would drift.
 *
 * THREE PROPERTIES THIS FILE EXISTS TO GUARANTEE
 *
 *  1. ROUND-TRIP IS BYTE-EXACT. `parseBrief` never loses, reflows or normalizes
 *     a line: the document is modelled as its own array of lines plus INDEXES
 *     into it, so `serializeBrief(parseBrief(text)) === text` for any input at
 *     all. A test pins this against the repo's real briefs, which are long,
 *     emoji-laden and full of nested markdown. If this were merely "usually
 *     true", every archive move would be a chance to corrupt the user's brief.
 *
 *  2. A MOVE ONLY MOVES. `moveEntry`/`removeEntry`/`insertEntry` splice whole
 *     lines from one place to another. The user's hand-written wording — the
 *     authoritative thing in a brief — comes out the other side of a move
 *     character for character, because nothing here ever rewrites a line.
 *
 *  3. EVERY ENTRY BECOMES A CARD. `deriveCard` has no failure mode that
 *     produces nothing: a retraction check, else a short bolded span, else a
 *     cut at the first em-dash or sentence end, else a truncated first line. A
 *     brief entry that renders as a blank card, or does not render at all, is
 *     worse than an ugly one — the board's whole claim is that it shows you the
 *     entire brief. One real entry bundles three unrelated fixes into a single
 *     bullet and no title can represent it; an awkward title is the right
 *     outcome there, and the rule is deliberately not contorted to chase it.
 *
 * THE SYNTHESIS SEAM. A separate layer (not built here) may precompute nicer
 * `{title, status, refs}` into a sidecar `.workspacer/brief.index.json` keyed by
 * a hash of the entry text. `cardFor` reads that index WHEN PRESENT and falls
 * back to `deriveCard` per-entry when it is absent, stale or missing a row — so
 * the board works today with no index file at all, and gains better titles later
 * without a line of rendering changing. `entryId` is that hash.
 *
 * The deterministic path is not a stopgap. A scout pass over the real briefs
 * measured it at roughly nine entries in ten reading well with no model
 * involved, and this codebase has no lightweight completion primitive to reach
 * for anyway — every "cheap model" pattern here spawns a whole agent session,
 * which is absurd for a card title. So this IS the shipping path, and it gets
 * the care the indexed path would have got.
 */

/** The status vocabulary. Deliberately `/standup`'s four (see
 *  renderer/lib/fleetManager.ts and services/managerSkills.ts) rather than a
 *  second enum for the same concept — the board is standup rendered
 *  persistently, so it must speak standup. */
export const BRIEF_STATUSES = ['in-flight', 'waiting-on-you', 'landed', 'next-up'] as const;
export type BriefStatus = (typeof BRIEF_STATUSES)[number];

/** The labels are `/standup`'s own section headings, verbatim (SKILL body in
 *  services/managerSkills.ts) — the board is standup rendered persistently, so
 *  it must not re-case or re-word the four states it shares with it. */
export const BRIEF_STATUS_LABELS: Record<BriefStatus, string> = {
  'in-flight': 'In flight',
  'waiting-on-you': 'Waiting on you',
  landed: 'Landed recently',
  'next-up': 'Next up',
};

/** The board's columns. The first three ARE the brief's own `##` sections, so a
 *  column move is a line moving between two headings the doctrine already
 *  defines — no new state, no new file format. `archive` is the fourth column
 *  and the only one that leaves the file, into `brief.archive.md`. */
export const BOARD_COLUMNS = ['Now', 'Direction', 'Recently', 'archive'] as const;
export type BoardColumn = (typeof BOARD_COLUMNS)[number];

export const BOARD_COLUMN_LABELS: Record<BoardColumn, string> = {
  Now: 'Now',
  Direction: 'Direction',
  Recently: 'Recently',
  archive: 'Archive',
};

/** Sections whose newest entry goes at the TOP: `## Recently` is a dated log
 *  kept newest-first. A card dropped there lands at the top; a card dropped
 *  anywhere else lands at the bottom of its section.
 *
 *  THE ONE COPY. briefService's appendToBrief needs the same rule to decide
 *  where `brief_append` inserts, and briefBoardService's archiveOldestEntries
 *  needs it to decide which END of a section holds the OLDEST entries. It used
 *  to be written out twice and kept in agreement by hand; the two spellings
 *  disagreeing would put the same entry in two different places depending on
 *  which door it came through, and would make an archive sweep take the newest
 *  entries instead of the oldest. Matching is case-insensitive because every
 *  other heading lookup here already is. */
const PREPEND_SECTIONS = new Set<string>(['recently']);

/** True when `name` is a section written newest-first. */
export function isPrependSection(name: string): boolean {
  return PREPEND_SECTIONS.has(
    String(name ?? '')
      .trim()
      .toLowerCase(),
  );
}

// ── Document model ───────────────────────────────────────────────────────────

/** One `#{2,6}` heading and the body beneath it, as line indexes. A body ends
 *  at the next heading of ANY level — the same boundary briefService uses, so
 *  the board and `brief_append` insert in the same place. */
export interface BriefSectionBlock {
  title: string;
  level: number;
  /** Index of the heading line itself. */
  headingLine: number;
  /** First body line (exclusive of the heading). */
  bodyStart: number;
  /** One past the last body line. */
  bodyEnd: number;
  /** The enclosing `##` section's title — itself when level is 2. A `###`
   *  sub-heading's entries belong to their parent column on the board, but
   *  splice against their own block. */
  column: string;
}

/** A top-level bullet and any continuation lines under it. */
export interface BriefEntry {
  /** Stable hash of the entry's exact text. The sidecar index is keyed by this,
   *  and it survives a move (the text does not change) but not an edit (which
   *  is the point — an edited entry's synthesized card is stale). */
  id: string;
  /** The `##` section this entry displays under. */
  column: string;
  /** The `###` sub-heading it sits beneath, when there is one. */
  group?: string;
  /** First line index (the bullet). */
  start: number;
  /** One past the last line index. */
  end: number;
  /** The entry's lines, verbatim. */
  lines: string[];
  /** `lines` joined — what the card is derived from and what gets hashed. */
  text: string;
}

export interface BriefDoc {
  /** The file split on '\n'. `join('\n')` restores it exactly, trailing newline
   *  or not, CRLF or not (a '\r' rides along at the end of its line). */
  lines: string[];
  sections: BriefSectionBlock[];
  entries: BriefEntry[];
}

const HEADING_RE = /^(#{1,6})\s+(.*?)\s*$/;
/** A top-level list bullet: `-`, `*`, `+`, `1.` or `1)`, with at most three
 *  leading spaces (four would be an indented continuation, per CommonMark). */
const BULLET_RE = /^ {0,3}(?:[-*+]|\d+[.)])\s+\S/;

/** True for a line that opens a new entry. Exported for the tests that pin the
 *  boundary rules against the real briefs. */
export function isEntryStart(line: string): boolean {
  return BULLET_RE.test(line) && !HEADING_RE.test(line);
}

function isHeading(line: string): boolean {
  return /^#{1,6}\s+\S/.test(line);
}

/**
 * Parse a brief into sections and entries WITHOUT consuming it: every line
 * stays in `lines`, and everything else is an index into that array.
 */
export function parseBrief(content: string): BriefDoc {
  const lines = content.split('\n');
  const sections: BriefSectionBlock[] = [];

  for (let i = 0; i < lines.length; i++) {
    const m = HEADING_RE.exec(lines[i]);
    if (!m || m[1].length < 2) continue; // the document's own `# Title` is not a section
    let end = lines.length;
    for (let j = i + 1; j < lines.length; j++) {
      if (isHeading(lines[j])) {
        end = j;
        break;
      }
    }
    const level = m[1].length;
    // The enclosing `##`. Walking backwards costs nothing at brief scale and
    // means a `####` under a `###` still resolves to the right column.
    let column = m[2];
    if (level > 2) {
      for (let k = sections.length - 1; k >= 0; k--) {
        if (sections[k].level < level) {
          column = sections[k].column;
          break;
        }
      }
    }
    sections.push({ title: m[2], level, headingLine: i, bodyStart: i + 1, bodyEnd: end, column });
  }

  const entries: BriefEntry[] = [];
  for (const sec of sections) {
    for (let i = sec.bodyStart; i < sec.bodyEnd; i++) {
      if (!isEntryStart(lines[i])) continue;
      // Continuations: indented or plain prose lines that follow. A blank line,
      // a new bullet or a heading closes the entry — so a blank separator the
      // author put between entries stays with the SECTION and never travels
      // with a card.
      let end = i + 1;
      while (
        end < sec.bodyEnd &&
        lines[end].trim() !== '' &&
        !isEntryStart(lines[end]) &&
        !isHeading(lines[end])
      ) {
        end++;
      }
      const slice = lines.slice(i, end);
      const text = slice.join('\n');
      entries.push({
        id: entryId(text),
        column: sec.column,
        group: sec.level > 2 ? sec.title : undefined,
        start: i,
        end,
        lines: slice,
        text,
      });
      i = end - 1;
    }
  }

  return { lines, sections, entries };
}

/** The inverse of `parseBrief`. Byte-exact by construction. */
export function serializeBrief(doc: BriefDoc): string {
  return doc.lines.join('\n');
}

/**
 * A stable content hash, as 16 lowercase hex chars. Two independently seeded
 * FNV-1a passes concatenated: the point is a cheap identifier that is identical
 * in every process that hashes the same entry (the sidecar index will be written
 * by a different one), not a cryptographic digest.
 */
export function entryId(text: string): string {
  const fnv = (seed: number): string => {
    let h = seed >>> 0;
    for (let i = 0; i < text.length; i++) {
      h ^= text.charCodeAt(i) & 0xff;
      h = Math.imul(h, 0x01000193) >>> 0;
      // Surrogate-safe: fold the high byte in too, or every emoji in these
      // briefs would hash the same as its low byte.
      h ^= (text.charCodeAt(i) >>> 8) & 0xff;
      h = Math.imul(h, 0x01000193) >>> 0;
    }
    return h.toString(16).padStart(8, '0');
  };
  return fnv(0x811c9dc5) + fnv(0x9dc5811c);
}

// ── Mutation: the only two things the board may do to a brief ────────────────

/** Remove an entry's lines. Returns the new line array; the caller re-parses. */
export function removeEntryLines(lines: string[], entry: BriefEntry): string[] {
  const next = lines.slice();
  next.splice(entry.start, entry.end - entry.start);
  return next;
}

/**
 * Where a new entry goes in `column`, given a freshly parsed doc. Mirrors
 * briefService.appendToBrief exactly: `Recently` prepends below any blank the
 * author left under the heading, everything else appends after the section's
 * last non-blank line. Returns -1 when the column has no heading.
 */
export function insertPointFor(doc: BriefDoc, column: string): number {
  const sec = doc.sections.find(
    (s) => s.level === 2 && s.title.toLowerCase() === column.toLowerCase(),
  );
  if (!sec) return -1;
  if (isPrependSection(sec.title) || isPrependSection(column)) {
    let at = sec.bodyStart;
    while (at < sec.bodyEnd && doc.lines[at].trim() === '') at++;
    return at;
  }
  let at = sec.bodyEnd;
  while (at > sec.bodyStart && doc.lines[at - 1].trim() === '') at--;
  return at;
}

export class BriefEntryNotFound extends Error {
  constructor(id: string) {
    super(`brief board: no entry ${id} in this brief (it moved or was edited) — reload the board`);
    this.name = 'BriefEntryNotFound';
  }
}

export class BriefColumnMissing extends Error {
  constructor(column: string) {
    super(`brief board: this brief has no "## ${column}" section`);
    this.name = 'BriefColumnMissing';
  }
}

/**
 * Move the entry identified by `id` into `column`, returning the new content.
 * A no-op (same content back) when the entry is already there — dropping a card
 * on the column it came from must not churn the file.
 *
 * The entry's lines are spliced out and spliced back in unchanged. That is the
 * whole implementation, and it is why a move cannot corrupt wording.
 */
export function moveEntryToColumn(content: string, id: string, column: string): string {
  const doc = parseBrief(content);
  const entry = doc.entries.find((e) => e.id === id);
  if (!entry) throw new BriefEntryNotFound(id);
  if (entry.column.toLowerCase() === column.toLowerCase() && !entry.group) return content;

  const withoutEntry = removeEntryLines(doc.lines, entry);
  const reparsed = parseBrief(withoutEntry.join('\n'));
  const at = insertPointFor(reparsed, column);
  if (at < 0) throw new BriefColumnMissing(column);

  const next = withoutEntry.slice();
  next.splice(at, 0, ...entry.lines);
  return next.join('\n');
}

/** Remove the entry entirely (the brief half of an archive move). */
export function removeEntry(content: string, id: string): { content: string; entry: BriefEntry } {
  const doc = parseBrief(content);
  const entry = doc.entries.find((e) => e.id === id);
  if (!entry) throw new BriefEntryNotFound(id);
  return { content: removeEntryLines(doc.lines, entry).join('\n'), entry };
}

const ARCHIVE_SKELETON =
  '# Brief archive\n\nCold storage for entries pruned from the brief — never rewritten, only appended.\n';

/**
 * Append an entry's lines to `brief.archive.md` under a `## <date>` batch
 * heading, creating the file or the heading as needed.
 *
 * APPEND-ONLY, per the /checkpoint doctrine: an existing line in the archive is
 * never touched, so a batch heading that is already there gets the entry added
 * at its end rather than the file being reorganized around it.
 */
export function appendToArchive(archive: string, entryLines: string[], date: string): string {
  const base = archive.trim() === '' ? ARCHIVE_SKELETON : archive;
  const lines = base.split('\n');

  let headingAt = -1;
  for (let i = 0; i < lines.length; i++) {
    const m = HEADING_RE.exec(lines[i]);
    if (m && m[1].length === 2 && m[2].trim() === date) {
      headingAt = i;
      break;
    }
  }

  if (headingAt < 0) {
    const trailing = lines[lines.length - 1] === '' ? '' : '\n';
    return `${base}${trailing}\n## ${date}\n${entryLines.join('\n')}\n`;
  }

  let end = lines.length;
  for (let i = headingAt + 1; i < lines.length; i++) {
    if (isHeading(lines[i])) {
      end = i;
      break;
    }
  }
  let at = end;
  while (at > headingAt + 1 && lines[at - 1].trim() === '') at--;
  const next = lines.slice();
  next.splice(at, 0, ...entryLines);
  return next.join('\n');
}

// ── Card derivation (the degraded path — always produces a card) ─────────────

/** What the synthesis layer will precompute per entry. Every field optional:
 *  a partial row is merged over the derived card rather than replacing it. */
export interface BriefIndexCard {
  title?: string;
  status?: string;
  summary?: string;
  refs?: string[];
}

/** `.workspacer/brief.index.json`, keyed by `entryId`. Both the wrapped shape
 *  and a bare map are accepted — the file is another layer's to define, and the
 *  board refusing to read it over an envelope detail would be the tail wagging
 *  the dog. */
export interface BriefIndexFile {
  version?: number;
  cards?: Record<string, BriefIndexCard>;
}

export type BriefIndex = Record<string, BriefIndexCard>;

/** Normalize whatever was in the sidecar into a flat map. Anything unrecognized
 *  yields `{}` — a malformed index degrades the board to derived cards, it does
 *  not break it. */
export function normalizeIndex(raw: unknown): BriefIndex {
  if (!raw || typeof raw !== 'object') return {};
  const obj = raw as Record<string, unknown>;
  const cards = obj.cards && typeof obj.cards === 'object' ? obj.cards : obj;
  const out: BriefIndex = {};
  for (const [k, v] of Object.entries(cards as Record<string, unknown>)) {
    if (!v || typeof v !== 'object') continue;
    const c = v as Record<string, unknown>;
    const row: BriefIndexCard = {};
    if (typeof c.title === 'string' && c.title.trim()) row.title = c.title.trim();
    if (typeof c.status === 'string') row.status = c.status;
    if (typeof c.summary === 'string' && c.summary.trim()) row.summary = c.summary.trim();
    if (Array.isArray(c.refs)) row.refs = c.refs.filter((r): r is string => typeof r === 'string');
    out[k] = row;
  }
  return out;
}

/** A card as the pane draws it. */
export interface BriefCard {
  id: string;
  column: BoardColumn;
  /** The `###` sub-heading the entry lives under, when any. */
  group?: string;
  title: string;
  /** Absent when nothing in the entry SAYS what state it is in. Deliberately
   *  not defaulted: see the note on `deriveStatus`. */
  status?: BriefStatus;
  /** The entry leads with a retraction/supersession marker (❌, WRONG,
   *  RETRACTED, SUPERSEDED). Surfaced because such an entry's bold span is the
   *  claim being DEBUNKED, and a card must not present it as the headline. */
  retracted?: boolean;
  /** A sentence or two after the title. May be empty. */
  summary: string;
  /** Session ids and commit shas mentioned in the entry. */
  refs: string[];
  /** A leading `YYYY-MM-DD`, when the entry carries one. */
  date?: string;
  /** The author's own leading glyph (✅ 🚧 ⚠️ …), kept as-is. */
  marker?: string;
  /** The entry verbatim — the card's tooltip and the "show the whole thing"
   *  expansion, so nothing on the board is a summary you cannot check. */
  text: string;
  /** True when a sidecar index row supplied the title/status. */
  synthesized: boolean;
  /** Archived cards are read-only: cold storage is append-only by doctrine. */
  archived?: boolean;
}

const BULLET_PREFIX_RE = /^ {0,3}(?:[-*+]|\d+[.)])\s+/;
/** Leading decoration: emoji, variation selectors, ZWJ sequences, whitespace. */
const MARKER_RE =
  /^(?:[\u{1F000}-\u{1FAFF}\u{2600}-\u{27BF}\u{2B00}-\u{2BFF}\u{FE0F}\u{200D}\u{20E3}\u{2190}-\u{21FF}\u{2B50}]+\s*)+/u;
const DATE_RE = /^(\d{4}-\d{2}-\d{2})[:.,]?\s+/;

/** Status keywords, checked in this order. `waiting-on-you` first because an
 *  entry can be both landed and blocking ("✅ RESOLVED … ONE HUMAN ACTION
 *  NEEDED"), and the one that needs the user is the one they must see. */
const STATUS_PATTERNS: Array<[BriefStatus, RegExp]> = [
  [
    'waiting-on-you',
    /\b(?:needs? (?:a |an )?(?:human|you\b|the user|decision|diagnosis|answer|rebuild)|human action|waiting on (?:you|the user)|awaiting|blocked on|escalat|user must|ask the user|for the user to|one human)\b/i,
  ],
  [
    'in-flight',
    // `dispatched` only counts when it is not being NEGATED — "NOT YET
    // DISPATCHED" is backlog language, and reading it as live work is the
    // single most likely misclassification in these briefs.
    /\b(?:in flight|in-flight|(?<!not yet )dispatched|dispatch(?:es)? out|in progress|underway|running now|being (?:built|written|fixed)|wip)\b/i,
  ],
  ['landed', /\b(?:resolved|fixed|merged|landed|shipped|committed|pushed|closed|built)\b/i],
  // "Next up" is scoped TIGHT on purpose. It is `/standup`'s own generated
  // suggestion rather than a thing brief entries say about themselves, so only
  // explicit backlog language claims it — anything looser would sweep every
  // un-landed entry into a state its author never asserted.
  [
    'next-up',
    /\b(?:not yet dispatched|next thing to build|to be dispatched|next up|backlog|queued|to dispatch|would dispatch next)\b/i,
  ],
];

/** The author's leading glyph, when it says something. These are the markers
 *  the briefs in this repo actually use. `📋`/`⭐`/`🐛` are NOT here: they mark a
 *  topic's kind (a design, a priority, a bug), not its state. */
const MARKER_STATUS: Array<[BriefStatus, RegExp]> = [
  ['waiting-on-you', /[⚠🚨❓❗]/u],
  ['in-flight', /[🚧🔨🏗]/u],
  ['landed', /[✅✔☑🎉]/u],
];

/**
 * Which state an entry's own words claim.
 *
 * `waiting-on-you` keeps ABSOLUTE precedence: an entry can be both landed and
 * blocking ("✅ RESOLVED … ONE HUMAN ACTION NEEDED"), and the one that needs
 * the user is the one they must see. Among the other three the EARLIEST mention
 * wins, not a fixed ranking — real entries mention several states in passing
 * ("Duplicate-wake bug FIXED and merged … it discarded the salvage that was
 * dispatched earlier"), and a fixed order lets a word buried in the detail
 * outvote the headline the author led with.
 */
function statusFromText(raw: string): BriefStatus | undefined {
  const [, blocking] = STATUS_PATTERNS[0];
  if (blocking.test(raw)) return 'waiting-on-you';
  let best: BriefStatus | undefined;
  let bestAt = Infinity;
  for (const [status, re] of STATUS_PATTERNS.slice(1)) {
    const at = raw.search(re);
    if (at >= 0 && at < bestAt) {
      bestAt = at;
      best = status;
    }
  }
  return best;
}

/**
 * Retraction / supersession, checked at the HEAD of the entry.
 *
 * A scout pass over the real briefs found an entry that opens
 * `❌ WRONG, RETRACTED SAME DAY…` whose only bold span is the debunked claim
 * itself. Trusting the bold span there would make the card assert, confidently
 * and in the headline position, the very falsehood the entry exists to correct.
 * So this is checked BEFORE the bold span, not after.
 */
const RETRACTION_RE = /^(?:[^A-Za-z0-9]{0,8})(?:WRONG|RETRACTED|SUPERSEDED|OBSOLETE|INCORRECT)\b/i;
const RETRACTION_GLYPH_RE = /[❌✖✗🚫]/u;

/** Parse a caller/index-supplied status. Unknown values fall through to the
 *  derived one — a card must never display a state the enum does not have. */
export function parseStatus(raw: unknown): BriefStatus | undefined {
  const want = String(raw ?? '')
    .trim()
    .toLowerCase()
    .replace(/[\s_]+/g, '-');
  const hit = BRIEF_STATUSES.find((s) => s === want);
  if (hit) return hit;
  const byLabel = (Object.keys(BRIEF_STATUS_LABELS) as BriefStatus[]).find(
    (s) => BRIEF_STATUS_LABELS[s].replace(/\s+/g, '-') === want,
  );
  return byLabel;
}

/** Strip markdown emphasis/code fences from a title fragment so the card header
 *  reads as text. The ENTRY is untouched — this only affects display. */
function plainify(s: string): string {
  return s
    .replace(/\*\*(.+?)\*\*/g, '$1')
    .replace(/(?<!`)`([^`]+)`(?!`)/g, '$1')
    .replace(/\*(.+?)\*/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  const cut = s.slice(0, max);
  const sp = cut.lastIndexOf(' ');
  return `${(sp > max * 0.6 ? cut.slice(0, sp) : cut).trimEnd()}…`;
}

const TITLE_MAX = 96;
const SUMMARY_MAX = 220;

/**
 * Cut at the first em-dash or sentence end — the third rule, and the one that
 * carries most entries that have no usable bold span. These briefs use ` — ` as
 * the headline/detail separator constantly, so it beats the period on average.
 * Abbreviations (`e.g.`, `i.e.`) are not treated as terminators.
 */
function headlineCut(s: string): string {
  const dash = /^(.{12,}?)\s+[—–]\s+/.exec(s);
  if (dash) return dash[1];
  const sentence = /^(.{20,}?[.!?])(?:\s|$)/.exec(s);
  if (sentence && !/\b(?:e\.g|i\.e|vs|etc|no|fig|approx)\.$/i.test(sentence[1])) return sentence[1];
  const nl = s.indexOf('\n');
  return nl > 0 ? s.slice(0, nl) : s;
}

/** The longest a bold span may be and still be a HEADLINE. Past this it is a
 *  bolded paragraph, and truncating it produces a worse title than cutting the
 *  prose at its first natural break. */
const BOLD_TITLE_MAX = 120;

/** How far into the entry a bold span may start and still be a HEADLINE.
 *  Mid-sentence emphasis is not a title: real briefs contain lines like
 *  "…right-click the card → **Terminate** → reopen from Overview", where the
 *  bold span is a UI label and taking it as the headline produces a card
 *  titled "Terminate" that says nothing about the entry. A headline sits at
 *  the front, after at most a short lead-in. */
const BOLD_TITLE_MAX_OFFSET = 12;

const REF_RE = /(session:[0-9a-f]{6,}|\b[0-9a-f]{7,12}\b)/g;
const MAX_REFS = 5;

function extractRefs(text: string): string[] {
  const out: string[] = [];
  // Only refs the author put in backticks or wrote as `session:<id>` count —
  // a bare hex run inside prose is far more often a number than a commit.
  for (const m of text.matchAll(/`([^`]+)`/g)) {
    for (const r of m[1].matchAll(REF_RE)) {
      if (!out.includes(r[1])) out.push(r[1]);
    }
  }
  for (const m of text.matchAll(/\bsession:[0-9a-f]{6,}\b/g)) {
    if (!out.includes(m[0])) out.push(m[0]);
  }
  return out.slice(0, MAX_REFS);
}

/**
 * The degraded card: everything derived from the entry text alone, no index, no
 * model. This is the path the board runs on today.
 */
export function deriveCard(entry: BriefEntry, column: BoardColumn): BriefCard {
  const raw = entry.text;
  let body = raw.replace(BULLET_PREFIX_RE, '');

  const markerMatch = MARKER_RE.exec(body);
  const marker = markerMatch ? markerMatch[0].trim() : undefined;
  if (markerMatch) body = body.slice(markerMatch[0].length);

  const dateMatch = DATE_RE.exec(body);
  const date = dateMatch ? dateMatch[1] : undefined;
  if (dateMatch) body = body.slice(dateMatch[0].length);

  // A second glyph can sit after the date (`- 2026-08-22 ✅ **…**`).
  const marker2 = MARKER_RE.exec(body);
  if (marker2) body = body.slice(marker2[0].length);

  // Title, in the corrected order: RETRACTION FIRST (a retracted entry's bold
  // span is the debunked claim, so it must not become the headline), then a
  // short bold span, then a cut at the first em-dash or sentence end, then the
  // character cap. Every rung ends in something non-empty.
  const plainHead = plainify(body).slice(0, 80);
  const retracted =
    RETRACTION_RE.test(plainHead) ||
    RETRACTION_GLYPH_RE.test(`${marker ?? ''}${marker2?.[0] ?? ''}`);

  let title = '';
  if (!retracted) {
    const bold = /\*\*(.+?)\*\*/s.exec(body);
    const boldText = bold ? plainify(bold[1]) : '';
    if (
      bold &&
      bold.index <= BOLD_TITLE_MAX_OFFSET &&
      boldText.length >= 8 &&
      boldText.length <= BOLD_TITLE_MAX
    ) {
      title = boldText;
    }
  }
  if (!title) title = plainify(headlineCut(plainify(body)));
  if (!title) title = plainify(body.split('\n')[0]);
  if (!title) title = plainify(raw) || '(empty entry)';
  title = truncate(title, TITLE_MAX);

  // Summary: what follows the title in the entry, so the card never just
  // repeats itself.
  const plainBody = plainify(body);
  const afterTitle = plainBody.startsWith(title.replace(/…$/, ''))
    ? plainBody.slice(title.replace(/…$/, '').length)
    : plainBody;
  const summary = truncate(afterTitle.replace(/^[\s—–\-:.,]+/, ''), SUMMARY_MAX);

  // STATUS COMES FROM THE ENTRY'S CONTENT, NEVER FROM WHICH SECTION IT SITS IN.
  // The obvious `## Now → in flight` mapping is wrong on the real data: a scout
  // sample found 6 of 65 `## Now` entries already resolved but unpruned, so a
  // section-derived status would relabel stale rot as live work — which is the
  // exact thing this board exists to clean up. An entry that says nothing about
  // its state therefore gets NO status rather than a guessed one.
  const glyphs = `${marker ?? ''}${marker2?.[0] ?? ''}`;
  const status =
    (glyphs ? MARKER_STATUS.find(([, re]) => re.test(glyphs))?.[0] : undefined) ??
    statusFromText(raw);

  return {
    id: entry.id,
    column,
    group: entry.group,
    title,
    status,
    retracted: retracted || undefined,
    summary,
    refs: extractRefs(raw),
    date,
    marker,
    text: raw,
    synthesized: false,
  };
}

/**
 * The card the board actually renders: derived, then overlaid with whatever the
 * sidecar index knows. An absent index, an absent row, or a row missing a field
 * all fall through to the derived value — never to a blank.
 */
export function cardFor(entry: BriefEntry, column: BoardColumn, index?: BriefIndex): BriefCard {
  const base = deriveCard(entry, column);
  const row = index?.[entry.id];
  if (!row) return base;
  const status = parseStatus(row.status) ?? base.status;
  const title = row.title ? truncate(plainify(row.title), TITLE_MAX) : base.title;
  return {
    ...base,
    title,
    status,
    summary: row.summary ? truncate(plainify(row.summary), SUMMARY_MAX) : base.summary,
    refs: row.refs?.length ? row.refs.slice(0, MAX_REFS) : base.refs,
    synthesized: Boolean(row.title || row.status || row.summary),
  };
}

/** Canonical column for a section title; `undefined` for a section the board
 *  has no column for (`## User`, say) — those entries are shown in a lane's
 *  "other" list rather than silently dropped. */
export function columnForSection(title: string): BoardColumn | undefined {
  const t = title.trim().toLowerCase();
  return (BOARD_COLUMNS as readonly string[]).find(
    (c) => c !== 'archive' && c.toLowerCase() === t,
  ) as BoardColumn | undefined;
}

/** Stable within-column order: group same-status cards together (the design's
 *  "group by status"), keeping file order inside a group. Nothing is dropped
 *  and nothing is ranked by importance — the board shows the whole brief.
 *  Statusless cards sort last as their own group; they are still all there. */
const STATUS_ORDER: Record<BriefStatus, number> = {
  'waiting-on-you': 0,
  'in-flight': 1,
  'next-up': 2,
  landed: 3,
};

export function statusRank(status?: BriefStatus): number {
  return status ? STATUS_ORDER[status] : 4;
}

export function sortCards(cards: BriefCard[]): BriefCard[] {
  return cards
    .map((c, i) => ({ c, i }))
    .sort((a, b) => statusRank(a.c.status) - statusRank(b.c.status) || a.i - b.i)
    .map((x) => x.c);
}

/** Build every card in a brief. Entries under a section with no column (e.g.
 *  `## User`) come back under `extras`, which the pane renders in a footer —
 *  "show everything" is a hard requirement, so there is no discard path. */
export function cardsForBrief(
  content: string,
  index?: BriefIndex,
): { cards: BriefCard[]; extras: BriefCard[] } {
  const doc = parseBrief(content);
  const cards: BriefCard[] = [];
  const extras: BriefCard[] = [];
  for (const entry of doc.entries) {
    const col = columnForSection(entry.column);
    if (col) cards.push(cardFor(entry, col, index));
    else extras.push({ ...cardFor(entry, 'Now', index), column: 'Now', group: entry.column });
  }
  return { cards: sortCards(cards), extras };
}

/** Cards for an archive file. Same derivation; the batch `## <date>` heading
 *  becomes the group, and every card is read-only. */
export function cardsForArchive(content: string, index?: BriefIndex): BriefCard[] {
  const doc = parseBrief(content);
  return doc.entries.map((entry) => ({
    ...cardFor(entry, 'archive', index),
    column: 'archive' as BoardColumn,
    group: entry.column,
    archived: true,
  }));
}
