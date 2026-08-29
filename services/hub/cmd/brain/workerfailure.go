package main

// Did the worker FINISH, or did it DIE? — the headless half of the one question
// a worker-finished wake could not answer.
//
// TWIN: apps/desktop/src/main/shared/workerFailure.ts, and pinned to the same
// cross-language fixture the Rust WRITER is pinned to
// (contracts/agent-error-marker-cases.json, loaded by workerfailure_test.go).
// Read the TypeScript file's header for the full history; the short version:
//
// A worker that dies on a provider error (out of credits, model overload, an
// upstream 5xx) goes idle exactly like one that finished, and its last assistant
// turn IS the error text. So the wake said "[fleet] Worker finished" and handed
// the manager the error string as the worker's summary — observed live on
// 2026-08-21. `stopped` (the SESSION ended) does not cover it: an error can
// arrive with the session still alive, and a SIGTERM'd worker is not the same
// event as one the API refused.
//
// Two signals, both already on the wire, neither invented here:
//
//  1. The agent-error MARKER. claudemon has no structured error field: its
//     managed-update fold (services/claudemon/src/providers/mod.rs,
//     AgentUpdate::Error) turns a provider error into an ordinary assistant turn
//     prefixed `⚠️ Error: `, deliberately, because the renderer only renders
//     known item kinds. That prefix is a real cross-process contract and is
//     pinned as one.
//  2. `status_line.overage_out_of_credits` — the daemon's structured
//     out-of-credits bit. This is standing ACCOUNT state, not a per-turn event:
//     it stays true for the whole session regardless of what the worker just
//     did, so it cannot by itself justify calling a turn a failure (observed
//     2026-08-22: four completed, correctly-merged dispatches all woke their
//     manager FAILED this way). It only ENRICHES a failure the marker already
//     established.
//
// FAIL-QUIET, not fail-loud: every check is a positive match on a known
// spelling, so an unrecognized failure degrades to being reported as a finish
// rather than mislabelling a genuine success as a crash. A false "finished" is
// the bug we already have; a false "FAILED" would be a new one.

import (
	"regexp"
	"strings"
	"unicode"
)

// agentErrorMarker is the prefix claudemon stamps on a provider-side error turn.
// TWIN: AGENT_ERROR_MARKER in workerFailure.ts and the `format!("⚠️ Error: …")`
// in providers/mod.rs. PINNED: contracts/agent-error-marker-cases.json.
const agentErrorMarker = "⚠️ Error: "

// failureReasonMax caps the reason text — a wake bullet must stay one line, and
// a long one crowds out everything else on it. TWIN: REASON_MAX.
const failureReasonMax = 200

// failureDashRe matches the bullet grammar's own ` — ` separator (and its
// ASCII/en-dash spellings) inside a reason. TWIN: the ` [—–-]{1,2} ` replace in
// flattenReason. Sanitized rather than escaped, so the wire grammar and its
// parser need no new quoting rule.
var failureDashRe = regexp.MustCompile(` [—–\-]{1,2} `)

// flattenFailureReason collapses whitespace, neutralizes the bullet separator
// and caps the result. TWIN: flattenReason in workerFailure.ts.
func flattenFailureReason(reason string) string {
	flat := strings.Join(strings.Fields(reason), " ")
	flat = failureDashRe.ReplaceAllString(flat, " - ")
	flat = strings.TrimSpace(flat)
	return clipRunes(flat, failureReasonMax, "…")
}

// errorMarkerReason returns the agent-error marker's message, and whether
// `text` is an error turn at all. TWIN: errorMarkerReason in workerFailure.ts.
//
// The marker must LEAD the message: prose that merely mentions an error, or a
// report that quotes one mid-message, is not a death — the fold only ever emits
// the marker at the start of a turn. And only the FIRST line is taken: a stack
// trace under the message is noise in a bullet, and claudemon's conversation
// store merges consecutive assistant_text items, so a mid-turn error and the
// turn's real reply arrive as ONE message (the fixture's last case).
func errorMarkerReason(text string) (string, bool) {
	trimmed := strings.TrimLeftFunc(text, unicode.IsSpace)
	if !strings.HasPrefix(trimmed, agentErrorMarker) {
		return "", false
	}
	first, _, _ := strings.Cut(strings.TrimPrefix(trimmed, agentErrorMarker), "\n")
	reason := flattenFailureReason(first)
	if reason == "" {
		// An empty reason must still read as a failure; a blank tail would
		// render as a bullet that says nothing happened.
		return "the provider reported an error with no message", true
	}
	return reason, true
}

// workerFailureReason says why this worker's turn ended in FAILURE, or reports
// an ordinary finish. `finalMessage` is the worker's last assistant turn — the
// same text the wake already carries as its report.
//
// Only the MARKER can create a failure, because only it is a per-turn event.
// `outOfCredits` is standing account state and says nothing about how THIS turn
// ended; once the marker has established a failure it is worth folding in,
// because it names the operator's actual problem rather than leaving it to the
// API's wording. TWIN: workerFailureReason in workerFailure.ts.
func workerFailureReason(outOfCredits bool, finalMessage string) (string, bool) {
	marker, ok := errorMarkerReason(finalMessage)
	if !ok {
		return "", false
	}
	if outOfCredits {
		return "out of credits (overage disabled) - " + marker, true
	}
	return marker, true
}
