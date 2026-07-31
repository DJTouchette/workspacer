/**
 * What counts as a genuine user send, in one place.
 *
 * ClaudePane dequeues its optimistic "Sending…" bubbles by counting how many
 * real user turns have arrived, so the definition has to match exactly what the
 * composer produces — and two things in a conversation look like a user turn
 * without being one. conversationApplier pushes a synthetic "nameless command
 * card" (role 'user', `command.name === ''`) for an orphaned command_output
 * whose invocation has already scrolled out of the window; dequeuing on that
 * would retract a bubble whose real turn has not landed yet.
 *
 * It lives in main/shared because THREE places need the same answer and they
 * sit on opposite sides of the IPC boundary: the main-process cap
 * (sessionStore/bounds), the renderer's background compaction
 * (lib/compactClaudeSnapshot), and the consumer (panes/ClaudePane). Both
 * trimmers bank the count of user sends they drop off the front so the consumer
 * can keep an ABSOLUTE tally; if the three ever disagreed about what a user
 * send is, that tally would drift by exactly the turns they disagreed on, and
 * the symptom would be a permanently duplicated message in a long session.
 */

/** The minimum shape the count needs. Structural on purpose: `bounds` is shared
 *  by both ingest paths and stays free of the renderer's snapshot types. */
export interface CountableTurn {
  role?: string;
  command?: { name?: string };
}

/** True when a turn is a message the user actually sent. */
export function isUserSend(turn: CountableTurn): boolean {
  return turn.role === 'user' && turn.command?.name !== '';
}

/** How many of these turns are genuine user sends. */
export function countUserSends(turns: readonly CountableTurn[]): number {
  let n = 0;
  for (const t of turns) if (isUserSend(t)) n++;
  return n;
}
