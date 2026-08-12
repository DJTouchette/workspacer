/**
 * Merge a compacted conversation WINDOW onto a retained full conversation.
 *
 * The bus publishes a bounded snapshot per flush (see hubTelemetry's
 * publishSnapshot): the newest N turns plus `conversationOffset`, the absolute
 * index of the first turn in that array. Absolute index = offset + array index,
 * the same contract `sessionStore/bounds.ts` and `compactClaudeSnapshot`
 * already maintain. That anchor is what lets a client holding the full history
 * splice a window in without the host ever resending the whole transcript.
 *
 * Every ambiguous case resolves toward NOT rendering a wrong transcript:
 * a gap asks the caller to refetch rather than silently concatenating across
 * missing turns, and a stale window is ignored rather than truncating history.
 * Rendering the wrong conversation for the session a user is watching is worse
 * than one extra fetch.
 */

export interface ConversationWindow<T> {
  conversation?: T[];
  conversationOffset?: number;
}

export type MergeOutcome<T> =
  /** Splice succeeded (or the window simply extended what we had). */
  | { kind: 'merged'; conversation: T[]; conversationOffset: number }
  /** The window starts at or before our history — it is authoritative on its own. */
  | { kind: 'adopt'; conversation: T[]; conversationOffset: number }
  /** The window starts after our history ends: turns are missing. Refetch. */
  | { kind: 'gap' }
  /** The window ends before our history does — an out-of-order/stale push. */
  | { kind: 'stale' };

/**
 * @param retained what the client currently shows (full history, or a previous merge)
 * @param incoming the compacted window just pushed
 */
export function mergeConversationWindow<T>(
  retained: ConversationWindow<T> | null | undefined,
  incoming: ConversationWindow<T>,
): MergeOutcome<T> {
  const windowTurns = incoming.conversation ?? [];
  const windowStart = incoming.conversationOffset ?? 0;
  const windowEnd = windowStart + windowTurns.length;

  // Nothing retained yet: the window is all we have.
  if (!retained || !retained.conversation) {
    return { kind: 'adopt', conversation: windowTurns, conversationOffset: windowStart };
  }

  const retainedTurns = retained.conversation;
  const retainedStart = retained.conversationOffset ?? 0;
  const retainedEnd = retainedStart + retainedTurns.length;

  // The offset moved BACKWARD. Offsets only ever advance as the host trims the
  // head, with exactly one exception: resetConversationOffsetIfRebuilt zeroes
  // them when the transcript is replaced and replayed from the top. So this is
  // a rebuild, and the replay is authoritative even though it is shorter than
  // what we hold — that is the point of a rebuild.
  if (windowStart < retainedStart) {
    return { kind: 'adopt', conversation: windowTurns, conversationOffset: windowStart };
  }

  // Same anchor: the window covers our whole history and possibly more.
  if (windowStart === retainedStart) {
    // ...unless it ends earlier, which means it is an older push that arrived
    // late. Adopting it would visibly delete turns the user can see.
    if (windowEnd < retainedEnd) return { kind: 'stale' };
    return { kind: 'adopt', conversation: windowTurns, conversationOffset: windowStart };
  }

  // The window begins after our history ends: at least one turn never reached
  // us (a dropped push, or more turns landed between flushes than the window
  // holds). Concatenating here would silently splice unrelated turns together.
  if (windowStart > retainedEnd) return { kind: 'gap' };

  // A late push that adds nothing we do not already have.
  if (windowEnd <= retainedEnd) return { kind: 'stale' };

  // Overlapping or exactly adjoining: keep our history up to where the window
  // begins, then take the window wholesale (its copies are the fresher ones —
  // a turn's content grows while it streams).
  const keep = retainedTurns.slice(0, windowStart - retainedStart);
  return {
    kind: 'merged',
    conversation: [...keep, ...windowTurns],
    conversationOffset: retainedStart,
  };
}
