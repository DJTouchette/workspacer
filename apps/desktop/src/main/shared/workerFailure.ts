/**
 * Did the worker FINISH, or did it DIE? — the one question a worker-finished
 * wake could not answer.
 *
 * A worker that dies on a provider error (out of credits, model overload, an
 * upstream 5xx) goes idle exactly like one that finished, and its last
 * assistant turn is the error text. So the wake said "[fleet] Worker finished"
 * and handed the manager the error string as the worker's summary — observed
 * live on 2026-08-21 with an out-of-credits worker, whose crash the manager had
 * every reason to record as a landed outcome.
 *
 * `27a881a2` already split "stopped/killed" (the session ENDED) out of a clean
 * finish. This extends that axis rather than duplicating it: stopped/killed
 * says the session went away, `failed` says the AGENT reported a failure — an
 * error can arrive with the session still alive, and a SIGTERM'd worker is not
 * the same event as one the API refused.
 *
 * Two signals, both already on the wire, neither invented here:
 *
 *  1. The agent-error MARKER. claudemon has no structured error field: its
 *     managed-update fold (providers/mod.rs, AgentUpdate::Error) turns a
 *     provider error into an ordinary assistant turn prefixed `⚠️ Error: `,
 *     deliberately, because the renderer only renders known item kinds. That
 *     prefix is therefore a real cross-process contract, and it is pinned as
 *     one: contracts/agent-error-marker-cases.json, loaded by the Rust WRITER's
 *     test and this module's READER test.
 *  2. `statusLine.overageOutOfCredits` — the daemon's structured
 *     out-of-credits bit (Claude stream `rate_limit_event`, overage disabled).
 *     Present on a session whose account has no headroom left, which is the
 *     exact case that prompted this, and it is worth naming explicitly rather
 *     than leaving to the error text's wording.
 *
 * FAIL-QUIET, not fail-loud: every check here is a positive match on a known
 * spelling, so an unrecognized failure degrades to today's behaviour (reported
 * as a finish) rather than mislabelling a genuine success as a crash. A false
 * "finished" is the bug we already have; a false "FAILED" would be a new one.
 */

import type { ClaudeSessionState } from '../services/claudeSessionStore';

/** The prefix claudemon stamps on a provider-side error turn.
 *  TWIN: services/claudemon/src/providers/mod.rs (AgentUpdate::Error).
 *  PINNED: contracts/agent-error-marker-cases.json. */
export const AGENT_ERROR_MARKER = '⚠️ Error: ';

/** Reason text longer than this is cut — a wake bullet must stay one line. */
const REASON_MAX = 200;

/** What a failure reason may not contain: the bullet's own ` — ` separator,
 *  and newlines. Sanitized rather than escaped so the wire grammar (and its
 *  parser) needs no new quoting rule. */
function flattenReason(reason: string): string {
  const flat = reason
    .replace(/\s+/g, ' ')
    .replace(/ [—–-]{1,2} /g, ' - ')
    .trim();
  return flat.length > REASON_MAX ? `${flat.slice(0, REASON_MAX)}…` : flat;
}

/**
 * The agent-error marker's message, or null when `text` is not an error turn.
 * The marker must LEAD the message: prose that merely mentions an error, or a
 * report that quotes one mid-message, is not a death — the fold only ever emits
 * the marker as the whole turn.
 */
export function errorMarkerReason(text: string): string | null {
  const trimmed = (text ?? '').trimStart();
  if (!trimmed.startsWith(AGENT_ERROR_MARKER)) return null;
  // Only the first line: a stack trace under the message is noise in a bullet.
  const first = trimmed.slice(AGENT_ERROR_MARKER.length).split('\n')[0];
  return flattenReason(first) || 'the provider reported an error with no message';
}

/** What the failure check needs from a session — the same partial shape the
 *  wake path already holds (a session evicted mid-window degrades cleanly). */
type MaybeFailedWorker = Partial<Pick<ClaudeSessionState, 'statusLine'>>;

/**
 * Why this worker's turn ended in FAILURE, or null for an ordinary finish.
 * `finalMessage` is the worker's last assistant turn — the same text the wake
 * already carries as its report.
 *
 * The out-of-credits bit wins over the marker text when both are present: it is
 * the structured signal, and it names the operator's actual problem (an account
 * with no headroom) rather than whatever wording the API returned.
 */
export function workerFailureReason(
  session: MaybeFailedWorker,
  finalMessage: string,
): string | null {
  const marker = errorMarkerReason(finalMessage);
  if (session.statusLine?.overageOutOfCredits === true) {
    return marker
      ? `out of credits (overage disabled) - ${marker}`
      : 'out of credits (overage disabled)';
  }
  return marker;
}
