/**
 * Bounds on live session state.
 *
 * These exist for two reasons, and the second is the expensive one:
 *
 *  1. Memory. A session's work log must not grow without bound if the events
 *     that normally clear it (Stop, a matching PostToolUse) never arrive.
 *
 *  2. IPC. Every coalesced flush sends the WHOLE session object to the renderer
 *     — `webContents.send` structured-clones it — and a streaming session
 *     flushes up to ~60 times a second. The cost of that clone is the size of
 *     this state, so anything unbounded here becomes an unbounded per-frame
 *     cost on the main event loop, which is the one thread that also forwards
 *     PTY bytes and services every other pane's IPC. A session that had read a
 *     few large files was measured at 6.3 MB and 3.1 ms per clone; local
 *     transcripts reach 14–23 MB.
 *
 * Shared by hookEventRouter (Claude's hook stream) and conversationApplier (the
 * managed-provider delta stream) so the two ingest paths can't drift.
 */

import { countUserSends, type CountableTurn } from '../../shared/conversationCount';

/** Keep only the most recent `max` entries, mutating in place. */
export function capInPlace<T>(arr: T[], max: number): void {
  if (arr.length > max) arr.splice(0, arr.length - max);
}

export const MAX_ACTIVE_TOOL_CALLS = 50;
export const MAX_COMPLETED_TOOL_CALLS = 50;

/**
 * The conversation is the one array a healthy session grows forever: every
 * message AND every tool call is its own turn, so an agent left running for a
 * day accumulates tens of thousands of them, each carrying a tool input and
 * response — and all of it is re-cloned to the renderer on every flush.
 *
 * This is a hard window, not a cache. A resync replays the daemon's transcript
 * back through applyConversationItems, which re-trims to this same cap, so
 * turns dropped here are permanently unreachable in the UI — ClaudePane's
 * "Load older messages" pager tops out here too. That is the reason the number
 * is thousands rather than the few hundred the memory argument alone would
 * suggest: at one turn per tool call a few hundred is well under an hour of
 * busy agent work, which would make losing scrollback routine instead of
 * exceptional. What keeps this many turns cheap to clone is the per-turn byte
 * budgets below (input + response); this count only bounds how many of them
 * there can be.
 */
export const MAX_CONVERSATION_TURNS = 2_000;

/**
 * The slice of a session the conversation bound touches, kept structural so
 * this module stays dependency-free for both ingest paths.
 *
 * `conversationOffset` is the running count of turns already dropped off the
 * front: absolute turn index = offset + array index. ClaudePane keys its rows
 * and anchors its changed-files snapshots by that absolute index, and the
 * renderer's background compaction (compactClaudeSnapshot) advances the same
 * field the same way — so any trim of the head must pay it forward or those
 * anchors silently point at the wrong turn. The renderer's snapshot type
 * (types/claudeSession.ts) declares it; ClaudeSessionState does not yet, which
 * is why the write goes through here rather than being spelled inline.
 *
 * `conversationUserOffset` is the same bookkeeping for the one count that is
 * NOT derivable from the turn offset: how many of the dropped turns were
 * genuine user sends. ClaudePane retires its optimistic "Sending…" bubbles by
 * watching the user-turn count grow, and a window-relative count SHRINKS when
 * the head is trimmed — which that code reads as "the thread reset", wiping
 * optimistic state and the answered-question cards. Banking the dropped user
 * sends lets it keep an absolute tally that only ever moves forward. The turn
 * offset cannot stand in for it because it counts turns of every role.
 */
export interface BoundedConversation {
  conversation: CountableTurn[];
  conversationOffset?: number;
  conversationUserOffset?: number;
}

/** Keep only the most recent MAX_CONVERSATION_TURNS turns, banking what was
 *  dropped into `conversationOffset` (and the user sends among them into
 *  `conversationUserOffset`) so absolute indices and counts stay correct. */
export function capConversationInPlace(session: BoundedConversation): void {
  const excess = session.conversation.length - MAX_CONVERSATION_TURNS;
  if (excess <= 0) return;
  const dropped = session.conversation.splice(0, excess);
  session.conversationOffset = (session.conversationOffset ?? 0) + excess;
  session.conversationUserOffset = (session.conversationUserOffset ?? 0) + countUserSends(dropped);
}

/**
 * An empty conversation means the next batch rebuilds history from the top:
 * both the `reset` delta and resyncConversation clear the array and replay the
 * daemon's whole transcript. Absolute indices restart at 0 there, so the offset
 * a previously-capped session had banked must not be carried into the replay —
 * it would be counted a second time as the rebuild re-trims.
 */
export function resetConversationOffsetIfRebuilt(session: BoundedConversation): void {
  if (session.conversation.length !== 0) return;
  session.conversationOffset = 0;
  session.conversationUserOffset = 0;
}

/** File changes feed the per-turn diff cards and the changed-files list. The
 *  renderer already tails this to 80 for background snapshots; this bounds what
 *  crosses the wire in the first place. */
export const MAX_FILE_CHANGES = 300;

/**
 * Tool results are stored verbatim, and a single Read of a large file can be
 * megabytes on its own — multiplied by every flush for the rest of the session.
 * Nothing renders more than a 4000-char excerpt (see ToolTraceCard's
 * excerptJson), so this keeps an ample margin over anything ever displayed
 * while removing the pathological case.
 */
export const MAX_TOOL_RESPONSE_CHARS = 32_768;

/** Truncate a tool result for storage, leaving a visible marker. Non-string
 *  payloads are left alone — they're small structured objects, and rewriting
 *  them would break the shape consumers switch on. */
export function truncateToolResponse(response: unknown): unknown {
  if (typeof response !== 'string' || response.length <= MAX_TOOL_RESPONSE_CHARS) return response;
  const dropped = response.length - MAX_TOOL_RESPONSE_CHARS;
  return `${response.slice(0, MAX_TOOL_RESPONSE_CHARS)}\n… [truncated ${dropped} chars]`;
}

/**
 * Inputs need the same bound as responses — a Write's `content`, an Edit's
 * new_string, a codex apply_patch diff are all whole files inlined into the
 * call, and they are stored for the life of the session, not just the turn.
 * Half the response budget because an input is a request, not a result: what
 * renders of it is a diff or an argv line.
 */
export const MAX_TOOL_INPUT_CHARS = 16_384;

/**
 * Strings this short are never the growth case — they're the identifying
 * fields (file_path, tool args, question text, todo lines) that main-process
 * consumers read structurally right after ingest, so they pass through whole
 * and don't spend budget. Trimming those to make room for a giant sibling
 * would corrupt a path to save nothing.
 */
const SMALL_INPUT_STRING_CHARS = 1_024;

/** Tool inputs come off the wire as parsed JSON, so this only guards against
 *  absurd nesting, not cycles. */
const MAX_INPUT_DEPTH = 8;

function spendOnString(value: string, budget: { left: number }): string {
  if (value.length <= SMALL_INPUT_STRING_CHARS) return value;
  const keep = Math.min(value.length, Math.max(0, budget.left));
  budget.left -= keep;
  if (keep === value.length) return value;
  return `${value.slice(0, keep)}\n… [truncated ${value.length - keep} chars]`;
}

function boundInputStrings(value: unknown, budget: { left: number }, depth: number): unknown {
  if (typeof value === 'string') return spendOnString(value, budget);
  if (value === null || typeof value !== 'object' || depth >= MAX_INPUT_DEPTH) return value;
  if (Array.isArray(value)) return value.map((v) => boundInputStrings(v, budget, depth + 1));
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
    out[k] = boundInputStrings(v, budget, depth + 1);
  }
  return out;
}

/**
 * Bound a tool call's input for storage. Unlike a response — which is opaque
 * text we can chop at the end — an input is read by key (file_path, todos,
 * questions, changes[].path) both here and in the renderer, so the object's
 * shape and keys must survive. Only the oversized string leaves are shortened,
 * against one shared budget spent in document order.
 */
export function truncateToolInput(input: unknown): unknown {
  if (typeof input === 'string') {
    return input.length <= MAX_TOOL_INPUT_CHARS
      ? input
      : `${input.slice(0, MAX_TOOL_INPUT_CHARS)}\n… [truncated ${input.length - MAX_TOOL_INPUT_CHARS} chars]`;
  }
  if (input === null || typeof input !== 'object') return input;
  // Measure before rewriting: the overwhelming majority of calls are a few
  // hundred bytes and must come back byref, not as a rebuilt copy.
  let size: number;
  try {
    size = JSON.stringify(input)?.length ?? 0;
  } catch {
    return input; // not wire-shaped data; not the growth case either
  }
  if (size <= MAX_TOOL_INPUT_CHARS) return input;
  return boundInputStrings(input, { left: MAX_TOOL_INPUT_CHARS }, 0);
}
