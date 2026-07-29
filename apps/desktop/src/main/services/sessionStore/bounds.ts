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

/** Keep only the most recent `max` entries, mutating in place. */
export function capInPlace<T>(arr: T[], max: number): void {
  if (arr.length > max) arr.splice(0, arr.length - max);
}

export const MAX_ACTIVE_TOOL_CALLS = 50;
export const MAX_COMPLETED_TOOL_CALLS = 50;

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
