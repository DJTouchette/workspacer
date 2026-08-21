/**
 * Fleet/supervisor system messages — the ONE place their wire format lives.
 *
 * supervisorNudge (main) injects wakes into a manager's conversation through
 * claudemon's plain-text /message endpoint, which means the text itself is the
 * only channel: it lands in the Claude transcript as an ordinary user turn and
 * no side-band metadata survives the round trip. The GUI still wants to render
 * these as structured cards, so the builder and the parser sit side by side
 * here — a format change that breaks parsing fails the round-trip tests in
 * this module instead of silently degrading the card back to a text blob.
 *
 * The entry list is line-based (`- ` bullets) precisely so parsing needs no
 * guesswork: labels and reply excerpts may contain `; `, parentheses, even
 * `session:` tokens, but excerpts are whitespace-flattened (see excerptReply)
 * so a bullet can never span lines.
 */

export type FleetMessageKind = 'worker-finished' | 'catch-up' | 'blocked';

export interface FleetMessageEntry {
  /** Worker label (or cwd-basename fallback). */
  label: string;
  sessionId: string;
  /** Working directory — present on finish/catch-up entries. */
  cwd?: string;
  /** What the agent is blocked on — present on 'blocked' entries. */
  blockedOn?: 'approval' | 'question';
  /** Flattened, capped excerpt of the worker's last reply. */
  lastReply?: string;
}

export interface FleetMessage {
  kind: FleetMessageKind;
  entries: FleetMessageEntry[];
}

/** Reply excerpts longer than this are cut (with an ellipsis). */
const REPLY_EXCERPT_MAX = 400;

/** One flattened, capped excerpt — keeps the wake readable AND single-line,
 *  which is what makes the bullet format parseable. */
export function excerptReply(reply: string): string {
  const flat = reply.replace(/\s+/g, ' ').trim();
  return flat.length > REPLY_EXCERPT_MAX ? `${flat.slice(0, REPLY_EXCERPT_MAX)}…` : flat;
}

/** Header lines. The bodies after them differ, so kind is the header alone. */
const HEADERS: Record<FleetMessageKind, string> = {
  'worker-finished': '[fleet] Worker finished:',
  'catch-up':
    '[fleet] Catch-up — these workers finished while you were idle and you may have missed the wake:',
  blocked: '[supervisor] An agent is now blocked on a decision:',
};

/** Instruction tails — what the manager/supervisor should DO with the wake. */
const TAILS: Record<FleetMessageKind, string> = {
  'worker-finished':
    `Review the result (get_conversation with sinceSeq for detail), append one line to that ` +
    `project's .workspacer/brief.md "## Recently" (and adjust "## Now"), then report the ` +
    `outcome briefly with session:<id> references. If it was not one of your dispatches, ` +
    `a one-line acknowledgement is enough.`,
  'catch-up':
    `Review each (get_conversation with sinceSeq), update the project brief's "## Recently", ` +
    `and report the outcome with session:<id> references. Then STOP again.`,
  blocked: `Run a /supervise pass: gather the context and notify me with a recommendation.`,
};

/** One entry as its bullet-body text (no leading `- `). */
export function formatFleetEntry(e: FleetMessageEntry): string {
  const where = e.blockedOn ? e.blockedOn : `cwd ${e.cwd || '?'}`;
  const tail = e.lastReply ? ` — last reply: ${e.lastReply}` : '';
  return `${e.label} (session:${e.sessionId}, ${where})${tail}`;
}

/** Compose a full wake message: header, one bullet per entry, instructions. */
export function buildFleetMessage(kind: FleetMessageKind, entries: FleetMessageEntry[]): string {
  const bullets = entries.map((e) => `- ${formatFleetEntry(e)}`);
  return `${HEADERS[kind]}\n${bullets.join('\n')}\n${TAILS[kind]}`;
}

/** Bullet-body grammar: `label (session:<id>, cwd <path>|approval|question)`
 *  with an optional ` — last reply: …` tail. Label is non-greedy so the FIRST
 *  `(session:` wins; a reply may contain anything (it is the anchored rest). */
const ENTRY_RE =
  /^(.+?) \(session:([\w-]+), (?:cwd (.+?)|(approval|question))\)(?: — last reply: (.*))?$/;

function parseEntry(body: string): FleetMessageEntry | null {
  const m = ENTRY_RE.exec(body);
  if (!m) return null;
  const [, label, sessionId, cwd, blockedOn, lastReply] = m;
  const e: FleetMessageEntry = { label, sessionId };
  if (cwd !== undefined) e.cwd = cwd;
  if (blockedOn) e.blockedOn = blockedOn as 'approval' | 'question';
  if (lastReply) e.lastReply = lastReply;
  return e;
}

/** Where a LEGACY (pre-bullet, single-paragraph) message's entry list ends. */
const LEGACY_TAIL_STARTS: Record<FleetMessageKind, string> = {
  'worker-finished': '. Review the result (get_conversation',
  'catch-up': '. Review each (get_conversation',
  blocked: '. Run a /supervise pass',
};

/**
 * Parse a legacy `<header> <e1>; <e2>. <instructions>` paragraph. The `; `
 * join was ambiguous (a reply excerpt can contain `; `), so segments that
 * don't parse as an entry are folded back into the previous entry's reply —
 * best-effort by design; anything unrecognizable falls back to raw text.
 */
function parseLegacyEntries(kind: FleetMessageKind, rest: string): FleetMessageEntry[] | null {
  const tailAt = rest.indexOf(LEGACY_TAIL_STARTS[kind]);
  if (tailAt < 0) return null;
  const list = rest.slice(0, tailAt).trim();
  const merged: string[] = [];
  for (const part of list.split('; ')) {
    if (merged.length > 0 && !parseEntry(part)) merged[merged.length - 1] += `; ${part}`;
    else merged.push(part);
  }
  const entries = merged.map(parseEntry);
  return entries.every((e): e is FleetMessageEntry => e !== null) && entries.length > 0
    ? entries
    : null;
}

/**
 * Recognize an injected fleet/supervisor wake in conversation text. Returns
 * null for anything that doesn't match EXACTLY — a card missing its fields is
 * worse than the raw-text fallback, so every bullet must parse or none do.
 */
export function parseFleetMessage(text: string): FleetMessage | null {
  const trimmed = text.trim();
  for (const kind of Object.keys(HEADERS) as FleetMessageKind[]) {
    const header = HEADERS[kind];
    if (!trimmed.startsWith(header)) continue;
    const rest = trimmed.slice(header.length);
    if (rest.startsWith('\n')) {
      // Current line-based format: bullets until the first non-bullet line.
      const lines = rest.split('\n').slice(1);
      const entries: FleetMessageEntry[] = [];
      for (const line of lines) {
        if (!line.startsWith('- ')) break;
        const e = parseEntry(line.slice(2));
        if (!e) return null;
        entries.push(e);
      }
      return entries.length > 0 ? { kind, entries } : null;
    }
    if (rest.startsWith(' ')) {
      const entries = parseLegacyEntries(kind, rest);
      return entries ? { kind, entries } : null;
    }
    return null;
  }
  return null;
}
