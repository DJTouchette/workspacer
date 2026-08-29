package main

// Fleet wake messages — the Go half of the ONE place their wire format lives.
//
// TWIN: apps/desktop/src/main/shared/fleetMessages.ts. Read that file's opening
// comment first: these wakes travel through claudemon's plain-text /message
// endpoint, so THE TEXT IS THE ONLY CHANNEL. It lands in the manager's
// transcript as an ordinary user turn and no side-band metadata survives the
// round trip. The GUI then re-parses that text back into a structured card
// (parseFleetMessage → FleetMessageCard), which means a header or a bullet this
// file spells differently does not degrade — it silently demotes the card to a
// text blob on every client, while the manager AGENT reads a header that no
// longer says what the doctrine trained it on.
//
// So this port is byte-exact for the two kinds the brain can produce, and it is
// PINNED to the TypeScript source by TestFleetMessageTwinMatchesTheDesktop
// (fleetmsg_test.go), which reads fleetMessages.ts and compares the strings
// rather than trusting this comment.
//
// ONLY TWO KINDS ARE PORTED, deliberately:
//
//   - progress  — agents.reportProgress, a worker's own mid-task line.
//   - threshold — agents.notifyWhen, a watch the manager armed firing once.
//
// worker-finished / catch-up / blocked are produced by the desktop's own
// supervisor nudge machinery (main/services/supervisorNudge.ts), which watches
// its own session store's finish transitions. The brain does not run that
// machinery, so porting their formats here would be dead code that could drift
// unnoticed — the two the brain can actually send are the two it spells.

import (
	"fmt"
	"strings"
)

// fleetProgressHeader / fleetThresholdHeader are HEADERS['progress'] and
// HEADERS['threshold'].
//
// The progress header deliberately does NOT contain the word "finished" and
// says STILL RUNNING in the header itself: the one failure mode of an
// unsolicited worker self-report is a manager booking it as an outcome.
const (
	fleetProgressHeader  = "[fleet] Progress update from a worker — it is STILL RUNNING; this is NOT a completion:"
	fleetThresholdHeader = "[fleet] A threshold you asked to be told about has been crossed:"
)

// fleetProgressTail / fleetThresholdTail are TAILS['progress'] and
// TAILS['threshold'] — what the manager should DO with the wake. The desktop
// composes each from adjacent string literals; these are the concatenations.
const (
	fleetProgressTail = "The worker sent this ITSELF, mid-task, and is still running: its finish wake will still " +
		"arrive. Do NOT record it in a brief's \"## Recently\" as work landed, and do not treat its " +
		"line as a result. If it needs no decision, do NOTHING — a bare acknowledgement costs the " +
		"worker a turn and tells it nothing. If it flags NEEDS A DECISION, or if the update shows " +
		"the dispatch going wrong, answer it with send_message, or stop it (signal SIGTERM, then " +
		"close_session) and redispatch surgically with respawn_with. Then STOP again — do not start " +
		"polling, and do not ask it for further updates."

	fleetThresholdTail = "This is the notify_when you armed, and it has now fired and been DISCARDED — watches are " +
		"one-shot, so nothing further will arrive unless you arm another. Decide: let it run, " +
		"send_message it a narrowing instruction, or stop it (signal SIGTERM, then close_session) " +
		"and redispatch surgically with respawn_with. Then STOP again — do not start polling."
)

// fleetEntry is the subset of FleetMessageEntry the brain's two kinds use.
// The fields it does NOT have (stopped / failed / lastReply / result /
// fullReply) belong to worker-finished wakes, which the brain does not send.
type fleetEntry struct {
	Label     string
	SessionID string
	Cwd       string
	// Crossed is the rendered threshold ("tokens 309,412 ≥ 250,000"), on
	// 'threshold' entries only.
	Crossed string
	// Note is the worker's own progress line, on 'progress' entries only.
	Note string
	// NeedsDecision says the worker is BLOCKED on the manager's answer rather
	// than merely keeping it informed. A rendering/urgency hint: the channel is
	// one-way and the manager still replies with send_message.
	NeedsDecision bool
}

// formatFleetEntry renders one entry's bullet BODY (no leading "- ").
//
// TWIN: formatFleetEntry in fleetMessages.ts, and the ORDER of the optional
// tails is load-bearing — ENTRY_RE on the parse side matches them in exactly
// this sequence, so swapping two makes the bullet unparseable rather than
// merely differently worded. The brain never emits the stopped/failed segments,
// which sit between `where` and `crossed`; leaving them out is the same string
// the desktop produces for an entry that has neither.
func formatFleetEntry(e fleetEntry) string {
	cwd := e.Cwd
	if cwd == "" {
		cwd = "?"
	}
	out := fmt.Sprintf("%s (session:%s, cwd %s)", e.Label, e.SessionID, cwd)
	if e.Crossed != "" {
		out += " — crossed: " + e.Crossed
	}
	if e.NeedsDecision {
		out += " — NEEDS A DECISION"
	}
	// One anchored tail, never two: the grammar has a single rest-of-line.
	if e.Note != "" {
		out += " — reports: " + e.Note
	}
	return out
}

// buildFleetMessage composes a wake: header, one bullet per entry, then the
// instruction tail. Neither of the brain's kinds carries the post-bullet extra
// blocks (FAILED/stopped notes, structured results, full replies), so the
// no-extras branch of the desktop's builder is the whole of this one.
func buildFleetMessage(header, tail string, entries []fleetEntry) string {
	bullets := make([]string, 0, len(entries))
	for _, e := range entries {
		bullets = append(bullets, "- "+formatFleetEntry(e))
	}
	return header + "\n" + strings.Join(bullets, "\n") + "\n" + tail
}

// fleetSenderHeaderPrefix / fleetSenderHeaderSuffix are SENDER_HEADER_PREFIX
// and SENDER_HEADER_SUFFIX in fleetMessages.ts — the two fixed halves of the
// agents.sendMessage attribution header, split out on both sides so the twin
// test can pin them against the TypeScript source text.
const (
	fleetSenderHeaderPrefix = "[fleet] session:"
	fleetSenderHeaderSuffix = " says:"
)

// fleetSenderHeaderText composes the attribution prefix a message whose caller
// named itself (agents.sendMessage's fromSessionId) is delivered with.
//
// TWIN: buildSenderHeader in fleetMessages.ts. Not a wake kind — the text that
// follows is arbitrary, so parseFleetMessage does not (and should not)
// round-trip it; it borrows the [fleet] / session:<id> vocabulary so the tokens
// read the same to the manager agent as a real wake's do. A sender with no
// recorded label is named by id alone rather than going unattributed, and there
// is deliberately no cwd-basename fallback: an invented label would read as a
// name the sender never had.
//
// The label LOOKUP lives with the metadata it reads (fleetSenderHeader in
// enrich.go); only the composition is here, where the desktop twin can pin it.
func fleetSenderHeaderText(sessionID, label string) string {
	named := sessionID
	if label != "" {
		named = sessionID + " (" + label + ")"
	}
	return fleetSenderHeaderPrefix + named + fleetSenderHeaderSuffix + "\n"
}

// fleetAgentLabel is the cwd-basename fallback for a worker with no label.
// TWIN: agentLabel in progressReports.ts and basename in thresholdWatch.ts —
// the same function under two names, because the wake bullets have to look the
// same whether the host or the worker itself produced them.
func fleetAgentLabel(cwd string) string {
	trimmed := strings.TrimRight(cwd, `/\`)
	if trimmed == "" {
		return ""
	}
	if i := strings.LastIndexAny(trimmed, `/\`); i >= 0 && i+1 < len(trimmed) {
		return trimmed[i+1:]
	}
	return trimmed
}
