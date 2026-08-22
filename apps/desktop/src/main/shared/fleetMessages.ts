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
  /** The worker's session ENDED (killed or exited) rather than idling — the
   *  wake says "stopped/killed" instead of reading as a clean finish. */
  stopped?: boolean;
  /** The worker DIED on a reported failure (a provider/API error, an
   *  out-of-credits refusal) rather than completing: the reason, already
   *  flattened to one line. A SEPARATE axis from `stopped`, which says the
   *  SESSION went away — an error can arrive with the session still alive, and
   *  a SIGTERM is not an API refusal. Round-tripped by the parser so the GUI
   *  card can badge it too. See shared/workerFailure. */
  failed?: string;
  /** A worker's VALIDATED structured result (pretty-printed JSON), when its
   *  dispatch carried a `resultSchema` and the worker honored the contract.
   *  Rendered as its own block below the bullets — builder-side only, like
   *  fullReply: the GUI card shows the prose excerpt, the object is for the
   *  manager agent, which can copy its fields into a brief line without
   *  re-deriving them from prose. */
  result?: string;
  /** Why no structured result could be read (block missing, unparseable, or
   *  schema-violating) for a dispatch that asked for one. Additive: the prose
   *  report is still delivered — this only says the machine-readable half did
   *  not arrive, so the manager knows to read the prose rather than assume the
   *  worker reported nothing. */
  resultError?: string;
  /** The worker's COMPLETE final assistant message, rendered as its own block
   *  below the bullet list (capped at FULL_REPLY_MAX with an explicit
   *  truncation note). Builder-side only: the parser does not round-trip it —
   *  the GUI card renders the excerpt; the full text is for the manager agent
   *  reading the raw wake. Set only when the excerpt is lossy. */
  fullReply?: string;
}

export interface FleetMessage {
  kind: FleetMessageKind;
  entries: FleetMessageEntry[];
}

/** Reply excerpts longer than this are cut (with an ellipsis). */
const REPLY_EXCERPT_MAX = 400;

/** Cap on a full-reply block. Deliberately generous — the point of carrying
 *  the complete final message is that the manager never has to fetch a 4KB
 *  report through get_conversation; a real report is a few KB, so tens of KB
 *  covers it with room, while still bounding a pathological reply. Truncation
 *  is announced in the block itself, never silent. */
export const FULL_REPLY_MAX = 32_768;

/** One flattened, capped excerpt — keeps the wake readable AND single-line,
 *  which is what makes the bullet format parseable. */
export function excerptReply(reply: string): string {
  const flat = reply.replace(/\s+/g, ' ').trim();
  return flat.length > REPLY_EXCERPT_MAX ? `${flat.slice(0, REPLY_EXCERPT_MAX)}…` : flat;
}

/** The full-reply block body: complete under the cap, explicitly annotated
 *  over it (keeping the head — reports lead with the outcome). */
function renderFullReply(reply: string): string {
  const text = reply.trim();
  if (text.length <= FULL_REPLY_MAX) return text;
  return (
    `${text.slice(0, FULL_REPLY_MAX)}\n` +
    `[truncated: showing the first ${FULL_REPLY_MAX} of ${text.length} characters — ` +
    `fetch the rest with get_conversation (lastMessage:true)]`
  );
}

/** Header lines. The bodies after them differ, so kind is the header alone. */
const HEADERS: Record<FleetMessageKind, string> = {
  'worker-finished': '[fleet] Worker finished:',
  'catch-up':
    '[fleet] Catch-up — these workers finished while you were idle and you may have missed the wake:',
  blocked: '[supervisor] An agent is now blocked on a decision:',
};

/** ALTERNATE headers a kind may be delivered under, parsed back to the same
 *  kind. `worker-finished` gets an honest one for the all-failed case: a wake
 *  whose every worker DIED must not open with the word "finished" — that is the
 *  exact sentence a manager read as a landed outcome. Mixed wakes keep the
 *  normal header and let the bullets carry the truth, because "finished" IS
 *  accurate for the entries that did. */
const ALT_HEADERS: Partial<Record<FleetMessageKind, string>> = {
  'worker-finished': '[fleet] Worker FAILED — did not complete:',
};

/** The header a given entry set is delivered under. */
function headerFor(kind: FleetMessageKind, entries: FleetMessageEntry[]): string {
  const alt = ALT_HEADERS[kind];
  if (alt && entries.length > 0 && entries.every((e) => e.failed)) return alt;
  return HEADERS[kind];
}

/** Instruction tails — what the manager/supervisor should DO with the wake. */
const TAILS: Record<FleetMessageKind, string> = {
  'worker-finished':
    `A "structured result" block below is the worker's own machine-readable report for a ` +
    `dispatch you gave a resultSchema — prefer its fields verbatim over re-deriving them ` +
    `from the prose. ` +
    `The worker's complete final message (when longer than its bullet excerpt) is included ` +
    `above — read it from this wake instead of fetching the conversation; use ` +
    `get_conversation (lastMessage:true for just the final message, or sinceSeq) only if you ` +
    `need more context. Append one line to that project's .workspacer/brief.md "## Recently" ` +
    `(and adjust "## Now"), then report the outcome briefly with session:<id> references. ` +
    `If it was not one of your dispatches, a one-line acknowledgement is enough.`,
  'catch-up':
    `Review each (get_conversation with sinceSeq), update the project brief's "## Recently", ` +
    `and report the outcome with session:<id> references. Then STOP again.`,
  blocked: `Run a /supervise pass: gather the context and notify me with a recommendation.`,
};

/** One entry as its bullet-body text (no leading `- `). */
export function formatFleetEntry(e: FleetMessageEntry): string {
  const where = e.blockedOn ? e.blockedOn : `cwd ${e.cwd || '?'}`;
  const stopped = e.stopped ? ' — stopped/killed' : '';
  const failed = e.failed ? ` — FAILED: ${e.failed}` : '';
  const tail = e.lastReply ? ` — last reply: ${e.lastReply}` : '';
  return `${e.label} (session:${e.sessionId}, ${where})${stopped}${failed}${tail}`;
}

/** Plain (non-bullet) note appended when any entry FAILED. Spelled out because
 *  the whole point is that a manager must not book a crash as an outcome. */
const FAILED_NOTE =
  `A "FAILED" entry did NOT complete its task — the agent reported an error (an API ` +
  `failure, an out-of-credits refusal, an overload) and stopped there. Its last reply is ` +
  `that error, NOT a result: do not record it in a brief's "## Recently" as work landed. ` +
  `Treat the dispatch as still open — re-dispatch it (respawn_with) or escalate the cause ` +
  `to the user if it is an account/quota problem no retry will fix.`;

/** Plain (non-bullet) note appended when any entry is stopped/killed. */
const STOPPED_NOTE =
  `A "stopped/killed" entry's session ENDED (killed or exited) rather than going idle — ` +
  `treat its last reply as possibly incomplete, not as a clean finish.`;

/**
 * Compose a full wake message: header, one bullet per entry, then (for
 * worker-finished wakes that carry one) each worker's COMPLETE final message
 * as its own block, then instructions. The extra blocks sit between the
 * bullets and the tail, where the parser's bullet loop has already stopped —
 * they are read by the manager agent, not round-tripped into the GUI card
 * (which shows the bullet excerpt).
 */
export function buildFleetMessage(kind: FleetMessageKind, entries: FleetMessageEntry[]): string {
  const bullets = entries.map((e) => `- ${formatFleetEntry(e)}`);
  const extras: string[] = [];
  if (entries.some((e) => e.failed)) extras.push(FAILED_NOTE);
  if (entries.some((e) => e.stopped)) extras.push(STOPPED_NOTE);
  for (const e of entries) {
    if (e.result) {
      extras.push(`Structured result — ${e.label} (session:${e.sessionId}):\n${e.result}`);
    } else if (e.resultError) {
      extras.push(
        `Structured result MISSING — ${e.label} (session:${e.sessionId}): ${e.resultError}. ` +
          `Read the prose report below/above instead.`,
      );
    }
  }
  for (const e of entries) {
    if (e.fullReply) {
      extras.push(
        `Full final message — ${e.label} (session:${e.sessionId}):\n${renderFullReply(e.fullReply)}`,
      );
    }
  }
  const head = `${headerFor(kind, entries)}\n${bullets.join('\n')}`;
  if (extras.length === 0) return `${head}\n${TAILS[kind]}`;
  return `${head}\n\n${extras.join('\n\n')}\n\n${TAILS[kind]}`;
}

/** Bullet-body grammar: `label (session:<id>, cwd <path>|approval|question)`
 *  with optional ` — stopped/killed` and ` — last reply: …` tails. Label is
 *  non-greedy so the FIRST `(session:` wins; a reply may contain anything (it
 *  is the anchored rest). */
const ENTRY_RE =
  /^(.+?) \(session:([\w-]+), (?:cwd (.+?)|(approval|question))\)(?: — (stopped\/killed))?(?: — FAILED: ((?:(?! — ).)+))?(?: — last reply: (.*))?$/;

function parseEntry(body: string): FleetMessageEntry | null {
  const m = ENTRY_RE.exec(body);
  if (!m) return null;
  const [, label, sessionId, cwd, blockedOn, stopped, failed, lastReply] = m;
  const e: FleetMessageEntry = { label, sessionId };
  if (cwd !== undefined) e.cwd = cwd;
  if (blockedOn) e.blockedOn = blockedOn as 'approval' | 'question';
  if (stopped) e.stopped = true;
  if (failed) e.failed = failed;
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
    // A kind may ship under more than one header (see ALT_HEADERS): the
    // all-failed spelling of worker-finished is still a worker-finished card,
    // so the parser must recognize both or the honest header would silently
    // demote the wake to a raw text blob in the GUI.
    const header = [HEADERS[kind], ALT_HEADERS[kind]].find(
      (h): h is string => !!h && trimmed.startsWith(h),
    );
    if (!header) continue;
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
