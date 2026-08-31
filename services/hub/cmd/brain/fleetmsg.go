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
// So this port is byte-exact for every kind the brain can produce, and it is
// PINNED to the TypeScript source by TestFleetMessageTwinMatchesTheDesktop
// (headlessport_test.go), which reads fleetMessages.ts and compares the strings
// rather than trusting this comment.
//
// ALL SIX KINDS ARE PORTED:
//
//   - progress        — agents.reportProgress, a worker's own mid-task line.
//   - threshold       — agents.notifyWhen, a watch the manager armed firing once.
//   - worker-finished — a dispatch coming home, produced by finishwake.go's own
//                       transition watcher over claudemon's /events stream.
//   - worker-escalated — the same terminal path, with a validated fixed-shape
//                        wks-escalation result and an honest non-completion face.
//   - catch-up        — the same wake under an honest header, re-sent by the
//                       backstop sweep when the live one never landed.
//   - blocked         — a worker STUCK on an approval or a question, broadcast
//                       to every live wake target by blockwake.go. The only
//                       kind that is not parent-keyed, and the only one whose
//                       bullet spends the `where` slot on the block kind
//                       instead of a cwd.
//
// Caller-authored `result` / `resultError` remain desktop-only because the
// brain declines resultSchema. Fixed host-authored escalation is ported below.

import (
	"fmt"
	"strings"
	"unicode/utf8"
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

	// fleetWorkerFinishedHeader is HEADERS['worker-finished'] and
	// fleetWorkerFailedHeader is ALT_HEADERS['worker-finished'] — the honest
	// spelling for an entry set whose every worker DIED. A wake that opens with
	// the word "finished" is the exact sentence a manager reads as a landed
	// outcome, so an all-failed set must not use it; a MIXED set keeps the
	// normal header and lets the bullets carry the truth, because "finished" IS
	// accurate for the entries that did. Both parse back to the same kind.
	fleetWorkerFinishedHeader  = "[fleet] Worker finished:"
	fleetWorkerFailedHeader    = "[fleet] Worker FAILED — did not complete:"
	fleetWorkerEscalatedHeader = "[fleet] Worker escalated — blocked and did not complete:"

	// fleetCatchUpHeader is HEADERS['catch-up'] — the backstop's spelling, which
	// says out loud that the manager may already have seen this.
	fleetCatchUpHeader = "[fleet] Catch-up — these workers finished while you were idle and you may have missed the wake:"

	// fleetBlockedHeader is HEADERS['blocked'] — the one header in this file
	// that opens `[supervisor]` rather than `[fleet]`. That is not a typo to be
	// tidied: the prefix is WIRE FORMAT, parseFleetMessage keys the card off it
	// on every client, and the manager doctrine was trained on this exact line.
	fleetBlockedHeader = "[supervisor] An agent is now blocked on a decision:"
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

	// fleetWorkerFinishedTail / fleetCatchUpTail are TAILS['worker-finished']
	// and TAILS['catch-up'].
	//
	// The worker-finished tail still names the "structured result" block even
	// though the brain never emits one (resultSchema is declined headless). That
	// is deliberate: it is the desktop's byte-exact string, and the ONE thing a
	// tail must do is read identically whichever host composed it — a manager
	// trained on one wording and handed another is the drift this twin exists to
	// prevent. The sentence is conditional in its own words ("a ... block below
	// is"), so a wake with no such block does not mislead.
	fleetWorkerFinishedTail = "A \"structured result\" block below is the worker's own machine-readable report for a " +
		"dispatch you gave a resultSchema — prefer its fields verbatim over re-deriving them " +
		"from the prose. " +
		"The worker's complete final message (when longer than its bullet excerpt) is included " +
		"above — read it from this wake instead of fetching the conversation; use " +
		"get_conversation (lastMessage:true for just the final message, or sinceSeq) only if you " +
		"need more context. Append one line to that project's .workspacer/brief.md \"## Recently\" " +
		"(and adjust \"## Now\"), then report the outcome briefly with session:<id> references. " +
		"If it was not one of your dispatches, a one-line acknowledgement is enough."

	fleetWorkerEscalatedTail = "This is a terminal escalation, NOT a completed outcome. Read the validated " +
		"\"worker escalation\" block above and do not record the task as landed in a brief's " +
		"\"## Recently\". Resolve or obtain the named authority/decision, then either answer the " +
		"worker with send_message or redispatch the remaining work with respawn_with. Keep the " +
		"dispatch open until the task actually completes."

	fleetCatchUpTail = "Review each (get_conversation with sinceSeq), update the project brief's \"## Recently\", " +
		"and report the outcome with session:<id> references. Then STOP again."

	// fleetBlockedTail is TAILS['blocked']. A single sentence where the others
	// are paragraphs, and it stays that way: it is the desktop's byte-exact
	// string, and the manager agent has to read the same instruction whichever
	// host composed the wake.
	fleetBlockedTail = "Run a /supervise pass: gather the context and notify me with a recommendation."
)

// fleetFailedNote / fleetStoppedNote are the plain (non-bullet) blocks a wake
// appends when any entry carries the matching flag. TWIN: FAILED_NOTE and
// STOPPED_NOTE in fleetMessages.ts. Spelled out rather than left to the bullet
// because the whole point is that a manager must not book a crash as an outcome.
const (
	fleetFailedNote = "A \"FAILED\" entry did NOT complete its task — the agent reported an error (an API " +
		"failure, an out-of-credits refusal, an overload) and stopped there. Its last reply is " +
		"that error, NOT a result: do not record it in a brief's \"## Recently\" as work landed. " +
		"Treat the dispatch as still open — re-dispatch it (respawn_with) or escalate the cause " +
		"to the user if it is an account/quota problem no retry will fix."

	fleetStoppedNote = "A \"stopped/killed\" entry's session ENDED (killed or exited) rather than going idle — " +
		"treat its last reply as possibly incomplete, not as a clean finish."
)

// fleetReplyExcerptMax is REPLY_EXCERPT_MAX: how much of a worker's last reply
// rides the BULLET. fleetFullReplyMax is FULL_REPLY_MAX, the cap on the
// complete-final-message block — deliberately generous, because the point of
// carrying the whole message is that the manager never has to fetch a 4KB report
// through get_conversation. Truncation is announced in the block, never silent.
const (
	fleetReplyExcerptMax = 400
	fleetFullReplyMax    = 32768
)

// excerptReply is the flattened, capped bullet excerpt. TWIN: excerptReply in
// fleetMessages.ts. Flattening is not cosmetic: the bullet grammar is LINE
// based, so a reply spanning lines makes the whole wake unparseable and the GUI
// card degrades to a text blob.
//
// The cap is counted in RUNES where the desktop counts UTF-16 code units. They
// agree on ASCII and on the BMP, and differ only in how many astral characters
// (emoji) fit before the ellipsis — a cosmetic difference in an excerpt that is
// explicitly lossy, and the alternative is cutting a multi-byte character in
// half, which is not.
func excerptReply(reply string) string {
	flat := flattenLine(reply)
	return clipRunes(flat, fleetReplyExcerptMax, "…")
}

// clipRunes cuts s to at most max runes, appending suffix when it had to cut.
func clipRunes(s string, max int, suffix string) string {
	if utf8.RuneCountInString(s) <= max {
		return s
	}
	n := 0
	for i := range s {
		if n == max {
			return s[:i] + suffix
		}
		n++
	}
	return s + suffix
}

// renderFullReply is the full-reply block BODY: complete under the cap,
// explicitly annotated over it (keeping the head — reports lead with the
// outcome). TWIN: renderFullReply in fleetMessages.ts.
//
// The character counts in the truncation note are runes for the same reason
// excerptReply's cap is, and the note is the only place the difference is
// visible: it names a number the manager might compare against the desktop's.
func renderFullReply(reply string) string {
	text := strings.TrimSpace(reply)
	total := utf8.RuneCountInString(text)
	if total <= fleetFullReplyMax {
		return text
	}
	return clipRunes(text, fleetFullReplyMax, "") + "\n" +
		fmt.Sprintf("[truncated: showing the first %d of %d characters — "+
			"fetch the rest with get_conversation (lastMessage:true)]", fleetFullReplyMax, total)
}

// fleetWorkerFinishedHeaderFor picks the header an entry set is delivered
// under. TWIN: headerFor in fleetMessages.ts, whose ALT_HEADERS table has this
// one entry.
func fleetWorkerFinishedHeaderFor(entries []fleetEntry) string {
	if len(entries) == 0 {
		return fleetWorkerFinishedHeader
	}
	for _, e := range entries {
		if e.Failed == "" {
			return fleetWorkerFinishedHeader
		}
	}
	return fleetWorkerFailedHeader
}

// fleetEntry is the subset of FleetMessageEntry the brain's kinds use.
//
// The two it does NOT have are `result` / `resultError`: those carry a
// VALIDATED structured result, and validation needs a resultSchema the headless
// spawn declines to accept (parity_test.go's spawnParamsDeclined), so a field
// here would have no writer.
type fleetEntry struct {
	Label     string
	SessionID string
	Cwd       string
	// BlockedOn is "approval" or "question", on 'blocked' entries only. It
	// REPLACES the cwd in the bullet's parenthesised slot rather than adding to
	// it — the desktop's grammar has one `where`, spelled either `cwd <path>` or
	// the block kind, and emitting both makes the bullet unparseable.
	BlockedOn string
	// Stopped says the worker's SESSION ended (killed or exited) rather than
	// going idle at its prompt: the bullet reads "stopped/killed" instead of as
	// a clean finish. On 'worker-finished' / 'catch-up' entries only.
	Stopped bool
	// Failed is why the worker DIED rather than completing — already flattened
	// to one line by workerFailureReason. A SEPARATE axis from Stopped, which
	// says the session went away: a provider error can arrive with the session
	// still alive, and a SIGTERM is not an API refusal.
	Failed string
	// Crossed is the rendered threshold ("tokens 309,412 ≥ 250,000"), on
	// 'threshold' entries only.
	Crossed string
	// Note is the worker's own progress line, on 'progress' entries only.
	Note string
	// NeedsDecision says the worker is BLOCKED on the manager's answer rather
	// than merely keeping it informed. A rendering/urgency hint: the channel is
	// one-way and the manager still replies with send_message.
	NeedsDecision bool
	// LastReply is the flattened, capped excerpt of the worker's final message,
	// on 'worker-finished' entries. Mutually exclusive with Note — the bullet
	// grammar has ONE rest-of-line, and a progress entry must never read as a
	// finish, so Note wins.
	LastReply string
	// FullReply is the worker's COMPLETE final message, rendered as its own
	// block below the bullets. Builder-side only (the desktop's parser does not
	// round-trip it): the card shows the excerpt, the full text is for the
	// manager AGENT, so it never has to fetch a conversation to read a report.
	// Set only when the excerpt is lossy.
	FullReply string
	// Escalation is the validated fixed-shape terminal response. Error is set
	// only when the tag existed but validation failed; that entry remains an
	// ordinary completion so malformed data is never silently accepted.
	Escalation      string
	EscalationError string
}

// formatFleetEntry renders one entry's bullet BODY (no leading "- ").
//
// TWIN: formatFleetEntry in fleetMessages.ts, and the ORDER of the optional
// tails is load-bearing — ENTRY_RE on the parse side matches them in exactly
// this sequence, so swapping two makes the bullet unparseable rather than
// merely differently worded.
func formatFleetEntry(e fleetEntry) string {
	// The `where` slot: the block kind on a 'blocked' entry, the cwd on every
	// other kind. TWIN: `const where = e.blockedOn ? e.blockedOn : \`cwd ${e.cwd || '?'}\``.
	where := e.BlockedOn
	if where == "" {
		cwd := e.Cwd
		if cwd == "" {
			cwd = "?"
		}
		where = "cwd " + cwd
	}
	out := fmt.Sprintf("%s (session:%s, %s)", e.Label, e.SessionID, where)
	if e.Stopped {
		out += " — stopped/killed"
	}
	if e.Failed != "" {
		out += " — FAILED: " + e.Failed
	}
	if e.Crossed != "" {
		out += " — crossed: " + e.Crossed
	}
	if e.NeedsDecision {
		out += " — NEEDS A DECISION"
	}
	// One anchored tail, never two: the grammar has a single rest-of-line, so
	// Note and LastReply are the same slot under two labels — and a worker's own
	// progress line WINS, because an entry carrying one is not a finish.
	if e.Note != "" {
		out += " — reports: " + e.Note
	} else if e.LastReply != "" {
		out += " — last reply: " + e.LastReply
	}
	return out
}

// buildFleetMessage composes a wake: header, one bullet per entry, then (for a
// worker-finished wake that carries them) the post-bullet extra blocks, then
// the instruction tail.
//
// The extras sit between the bullets and the tail, where the desktop parser's
// bullet loop has already stopped — they are read by the manager AGENT, not
// round-tripped into the GUI card, which renders the bullet excerpt. Their
// ORDER is the desktop's: the FAILED note, the stopped note, then one full-reply
// block per entry that has one. (The desktop emits structured-result blocks
// between those two groups; the brain never has one to emit, and the order of
// what remains is unchanged by their absence.)
//
// A progress or threshold entry sets none of these fields, so this composes the
// byte-identical no-extras string those two kinds always produced.
func buildFleetMessage(header, tail string, entries []fleetEntry) string {
	bullets := make([]string, 0, len(entries))
	for _, e := range entries {
		bullets = append(bullets, "- "+formatFleetEntry(e))
	}
	head := header + "\n" + strings.Join(bullets, "\n")

	var extras []string
	for _, e := range entries {
		if e.Failed != "" {
			extras = append(extras, fleetFailedNote)
			break
		}
	}
	for _, e := range entries {
		if e.Stopped {
			extras = append(extras, fleetStoppedNote)
			break
		}
	}
	for _, e := range entries {
		if e.Escalation != "" {
			extras = append(extras, fmt.Sprintf("Worker escalation — %s (session:%s):\n%s",
				e.Label, e.SessionID, e.Escalation))
		} else if e.EscalationError != "" {
			extras = append(extras, fmt.Sprintf("Worker escalation INVALID — %s (session:%s): %s. "+
				"The terminal marker was rejected; treat the prose as an ordinary completion or refusal.",
				e.Label, e.SessionID, e.EscalationError))
		}
	}
	for _, e := range entries {
		if e.FullReply != "" {
			extras = append(extras, fmt.Sprintf("Full final message — %s (session:%s):\n%s",
				e.Label, e.SessionID, renderFullReply(e.FullReply)))
		}
	}
	if len(extras) == 0 {
		return head + "\n" + tail
	}
	return head + "\n\n" + strings.Join(extras, "\n\n") + "\n\n" + tail
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
