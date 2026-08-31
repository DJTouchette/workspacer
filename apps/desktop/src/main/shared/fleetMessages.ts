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

export type FleetMessageKind =
  'worker-finished' | 'worker-escalated' | 'catch-up' | 'blocked' | 'threshold' | 'progress';

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
  /** The threshold this session crossed, already rendered ("tokens 309,412 ≥
   *  250,000"). Present on 'threshold' entries — the answer to a notify_when
   *  the manager armed so it would never have to poll. Round-tripped. */
  crossed?: string;
  /** A worker's OWN mid-task progress line, in its own words (flattened and
   *  capped by the host before it gets here). Present on 'progress' entries —
   *  the worker-initiated half that notify_when's host-side thresholds cannot
   *  cover: "finished phase 1", "the approach you gave me is wrong", "I'm
   *  reading more than I expected". Round-tripped, and rendered under its OWN
   *  label ("reports:") rather than lastReply's, because the entry is not a
   *  finish and must never read as one. Mutually exclusive with lastReply. */
  note?: string;
  /** The worker says it is BLOCKED on the manager's answer, not merely keeping
   *  it informed. Only a rendering/urgency hint — the channel is one-way, and
   *  the manager still replies with send_message. Round-tripped. */
  needsDecision?: boolean;
  /** A worker's VALIDATED structured result (pretty-printed JSON), when its
   *  dispatch carried a `resultSchema` and the worker honored the contract.
   *  Rendered as its own block below the bullets, and ROUND-TRIPPED: the
   *  manager agent reads the fields verbatim out of the wake text, and the GUI
   *  renders the same object as a structured card (StructuredResultCard).
   *  Unlike fullReply — which is prose the card already shows an excerpt of —
   *  dropping this on the parse side left the GUI with no trace at all of a
   *  report the worker was explicitly asked for. */
  result?: string;
  /** Why no structured result could be read (block missing, unparseable, or
   *  schema-violating) for a dispatch that asked for one. Additive: the prose
   *  report is still delivered — this only says the machine-readable half did
   *  not arrive, so the manager knows to read the prose rather than assume the
   *  worker reported nothing. Round-tripped beside `result`, so the card can
   *  say the contract was missed instead of quietly showing nothing. */
  resultError?: string;
  /** A validated fixed-shape `wks-escalation` terminal response. Independent
   *  from result/resultError: escalation works without resultSchema and a
   *  normal wks-result completion continues to use its existing slot. */
  escalation?: string;
  /** A tagged `wks-escalation` block was present but failed validation. This
   *  stays on an ordinary completion wake so malformed data is never silently
   *  promoted into the distinct worker-escalated outcome. */
  escalationError?: string;
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
  'worker-escalated': '[fleet] Worker escalated — blocked and did not complete:',
  'catch-up':
    '[fleet] Catch-up — these workers finished while you were idle and you may have missed the wake:',
  blocked: '[supervisor] An agent is now blocked on a decision:',
  threshold: '[fleet] A threshold you asked to be told about has been crossed:',
  // Deliberately does NOT contain the word "finished", and says STILL RUNNING
  // in the header itself: the one failure mode of an unsolicited worker
  // self-report is a manager booking it as an outcome. The card face differs
  // too (see FleetMessageCard) — but the header is what the manager AGENT
  // reads, and it is the line that has to be unmistakable.
  progress:
    '[fleet] Progress update from a worker — it is STILL RUNNING; this is NOT a completion:',
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
  'worker-escalated':
    `This is a terminal escalation, NOT a completed outcome. Read the validated ` +
    `"worker escalation" block above and do not record the task as landed in a brief's ` +
    `"## Recently". Resolve or obtain the named authority/decision, then either answer the ` +
    `worker with send_message or redispatch the remaining work with respawn_with. Keep the ` +
    `dispatch open until the task actually completes.`,
  'catch-up':
    `Review each (get_conversation with sinceSeq), update the project brief's "## Recently", ` +
    `and report the outcome with session:<id> references. Then STOP again.`,
  blocked: `Run a /supervise pass: gather the context and notify me with a recommendation.`,
  threshold:
    `This is the notify_when you armed, and it has now fired and been DISCARDED — watches are ` +
    `one-shot, so nothing further will arrive unless you arm another. Decide: let it run, ` +
    `send_message it a narrowing instruction, or stop it (signal SIGTERM, then close_session) ` +
    `and redispatch surgically with respawn_with. Then STOP again — do not start polling.`,
  progress:
    `The worker sent this ITSELF, mid-task, and is still running: its finish wake will still ` +
    `arrive. Do NOT record it in a brief's "## Recently" as work landed, and do not treat its ` +
    `line as a result. If it needs no decision, do NOTHING — a bare acknowledgement costs the ` +
    `worker a turn and tells it nothing. If it flags NEEDS A DECISION, or if the update shows ` +
    `the dispatch going wrong, answer it with send_message, or stop it (signal SIGTERM, then ` +
    `close_session) and redispatch surgically with respawn_with. Then STOP again — do not start ` +
    `polling, and do not ask it for further updates.`,
};

/** One entry as its bullet-body text (no leading `- `). */
export function formatFleetEntry(e: FleetMessageEntry): string {
  const where = e.blockedOn ? e.blockedOn : `cwd ${e.cwd || '?'}`;
  const stopped = e.stopped ? ' — stopped/killed' : '';
  const failed = e.failed ? ` — FAILED: ${e.failed}` : '';
  const crossed = e.crossed ? ` — crossed: ${e.crossed}` : '';
  const decision = e.needsDecision ? ' — NEEDS A DECISION' : '';
  // One anchored tail, never two: `note` and `lastReply` are the same slot
  // under different labels (the grammar can only have one rest-of-line), and a
  // worker's own progress line wins — an entry carrying one is not a finish.
  const tail = e.note
    ? ` — reports: ${e.note}`
    : e.lastReply
      ? ` — last reply: ${e.lastReply}`
      : '';
  return `${e.label} (session:${e.sessionId}, ${where})${stopped}${failed}${crossed}${decision}${tail}`;
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

/** Matches a reply reference this module produced, at the start of a user
 *  message — so a second Reply click replaces rather than stacks. */
export const REPLY_PREFIX_RE = /^Re: session:\S+ \([^)]*\) — /;

/** The two fixed halves of the sender-attribution header (see
 *  buildSenderHeader). Split out so the Go twin can be pinned to these exact
 *  literals by TestFleetMessageTwinMatchesTheDesktop, which compares against
 *  this file's source text and so cannot see through a template literal. */
export const SENDER_HEADER_PREFIX = '[fleet] session:';
export const SENDER_HEADER_SUFFIX = ' says:';

/**
 * Attribution for `agents.sendMessage` — the one fleet-chat message class with
 * no header of its own.
 *
 * Every other message a manager receives names its subject (a finish, a
 * threshold, a progress line all carry `session:<id>` in their bullets), but a
 * worker answering its manager with send_message arrived as bare text: the
 * manager could not tell WHICH worker was speaking, or that an agent was
 * speaking at all. `fromSessionId` is the caller saying who it is, and this is
 * what the recipient then reads.
 *
 * NOT a FleetMessageKind, deliberately: the text that follows is arbitrary, so
 * there is no entry grammar to parse and `parseFleetMessage` neither does nor
 * should round-trip it. It borrows `[fleet]` and `session:<id>` from the wake
 * vocabulary above so the tokens read the same to the manager agent and to a
 * human skimming the transcript.
 *
 * The label is the sender's spawn label when the host has one; a sender it
 * never recorded a label for is still named by id rather than going
 * unattributed. There is deliberately NO cwd-basename fallback here (unlike a
 * wake bullet's `agentLabel`) — the brain's twin has none either, and an
 * invented label would read as a name the sender never had.
 *
 * TWIN: fleetSenderHeaderText in services/hub/cmd/brain/fleetmsg.go.
 */
export function buildSenderHeader(sender: { sessionId: string; label?: string }): string {
  const named = sender.label ? `${sender.sessionId} (${sender.label})` : sender.sessionId;
  return `${SENDER_HEADER_PREFIX}${named}${SENDER_HEADER_SUFFIX}\n`;
}

/**
 * The composer-side half of threading's cheap 80% (see
 * .workspacer/threads-research-2026-08-22.md, §6): a pointer prepended to the
 * next message so the manager can tell which worker a reply is about, without
 * any protocol change — `session:<id>` is already the vocabulary the wake
 * bullets and the manager's doctrine both use.
 */
export function buildReplyPrefix(entry: Pick<FleetMessageEntry, 'sessionId' | 'label'>): string {
  return `Re: session:${entry.sessionId} (${entry.label}) — `;
}

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
    if (e.escalation) {
      extras.push(`Worker escalation — ${e.label} (session:${e.sessionId}):\n${e.escalation}`);
    } else if (e.escalationError) {
      extras.push(
        `Worker escalation INVALID — ${e.label} (session:${e.sessionId}): ${e.escalationError}. ` +
          `The terminal marker was rejected; treat the prose as an ordinary completion or refusal.`,
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
 *  with optional ` — stopped/killed` and ` — last reply: …`/` — reports: …`
 *  tails. Label is non-greedy so the FIRST `(session:` wins; the tail may
 *  contain anything (it is the anchored rest) — and the two spellings of that
 *  tail are alternatives, not siblings, because only one rest-of-line exists. */
const ENTRY_RE =
  /^(.+?) \(session:([\w-]+), (?:cwd (.+?)|(approval|question))\)(?: — (stopped\/killed))?(?: — FAILED: ((?:(?! — ).)+))?(?: — crossed: ((?:(?! — ).)+))?(?: — (NEEDS A DECISION))?(?: — (?:last reply: (.*)|reports: (.*)))?$/;

function parseEntry(body: string): FleetMessageEntry | null {
  const m = ENTRY_RE.exec(body);
  if (!m) return null;
  const [, label, sessionId, cwd, blockedOn, stopped, failed, crossed, decision, lastReply, note] =
    m;
  const e: FleetMessageEntry = { label, sessionId };
  if (cwd !== undefined) e.cwd = cwd;
  if (blockedOn) e.blockedOn = blockedOn as 'approval' | 'question';
  if (stopped) e.stopped = true;
  if (failed) e.failed = failed;
  if (crossed) e.crossed = crossed;
  if (decision) e.needsDecision = true;
  if (lastReply) e.lastReply = lastReply;
  if (note) e.note = note;
  return e;
}

/** Where a LEGACY (pre-bullet, single-paragraph) message's entry list ends. */
const LEGACY_TAIL_STARTS: Record<FleetMessageKind, string> = {
  'worker-finished': '. Review the result (get_conversation',
  'worker-escalated': '\u0000no legacy worker escalation wakes exist',
  'catch-up': '. Review each (get_conversation',
  blocked: '. Run a /supervise pass',
  // 'threshold' post-dates the bullet format entirely — no legacy paragraph of
  // this kind exists, and the sentinel must not be an empty string (which
  // indexOf finds at 0 in ANY text and would make every non-wake string parse).
  threshold: '\u0000no legacy threshold wakes exist',
  // Same story, same sentinel: 'progress' post-dates the bullet format too.
  progress: '\u0000no legacy progress wakes exist',
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

/** A structured-result block's head line, and the JSON body under it. The
 *  label is non-greedy for the same reason ENTRY_RE's is: the FIRST
 *  `(session:` wins. The body is taken verbatim — including the
 *  `[truncated: …]` marker readStructuredResult may have appended, which is
 *  exactly why the card must survive a body that is not valid JSON. */
const RESULT_BLOCK_RE = /^Structured result — .+? \(session:([\w-]+)\):\n([\s\S]+)$/;

/** The MISSING spelling: one line, reason in the middle. The reason is
 *  recovered as text, not byte-identically — a reason that itself ended in a
 *  period loses it to the sentence join. Nothing downstream compares it. */
const RESULT_MISSING_RE =
  /^Structured result MISSING — .+? \(session:([\w-]+)\): ([\s\S]+?)\. Read the prose report below\/above instead\.$/;

const ESCALATION_BLOCK_RE = /^Worker escalation — .+? \(session:([\w-]+)\):\n([\s\S]+)$/;
const ESCALATION_INVALID_RE =
  /^Worker escalation INVALID — .+? \(session:([\w-]+)\): ([\s\S]+?)\. The terminal marker was rejected; treat the prose as an ordinary completion or refusal\.$/;

/** Where the agent-facing free-form blocks start. buildFleetMessage emits every
 *  structured-result block BEFORE the first full-reply block, and a full reply
 *  is arbitrary worker prose that may contain blank lines (so its own
 *  paragraphs would be scanned as blocks). Stopping here keeps a worker that
 *  quotes this format inside its report from forging a result. */
const FULL_REPLY_MARK = 'Full final message — ';

/**
 * Fold the post-bullet blocks a wake may carry (see buildFleetMessage's
 * `extras`) back onto their entries. Best-effort and additive by construction:
 * an unrecognized block — the FAILED note, the stopped note, the instruction
 * tail — is simply skipped, so a wake still parses to the same card it did
 * before these blocks existed.
 */
function attachResultBlocks(tail: string[], entries: FleetMessageEntry[]): void {
  const byId = new Map(entries.map((e) => [e.sessionId, e]));
  for (const raw of tail.join('\n').split('\n\n')) {
    const block = raw.trim();
    if (block.startsWith(FULL_REPLY_MARK)) return;
    const ok = RESULT_BLOCK_RE.exec(block);
    if (ok) {
      const entry = byId.get(ok[1]);
      if (entry) entry.result = ok[2];
      continue;
    }
    const missing = RESULT_MISSING_RE.exec(block);
    if (missing) {
      const entry = byId.get(missing[1]);
      if (entry) entry.resultError = missing[2];
      continue;
    }
    const escalation = ESCALATION_BLOCK_RE.exec(block);
    if (escalation) {
      const entry = byId.get(escalation[1]);
      if (entry) entry.escalation = escalation[2];
      continue;
    }
    const invalidEscalation = ESCALATION_INVALID_RE.exec(block);
    if (invalidEscalation) {
      const entry = byId.get(invalidEscalation[1]);
      if (entry) entry.escalationError = invalidEscalation[2];
    }
  }
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
      let i = 0;
      for (; i < lines.length; i++) {
        if (!lines[i].startsWith('- ')) break;
        const e = parseEntry(lines[i].slice(2));
        if (!e) return null;
        entries.push(e);
      }
      if (entries.length === 0) return null;
      attachResultBlocks(lines.slice(i), entries);
      return { kind, entries };
    }
    if (rest.startsWith(' ')) {
      const entries = parseLegacyEntries(kind, rest);
      return entries ? { kind, entries } : null;
    }
    return null;
  }
  return null;
}
