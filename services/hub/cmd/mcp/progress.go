// report_progress — the one thing a dispatched worker may say to the manager
// that dispatched it.
//
// TIER. This tool is present at EVERY tier, including view, and that is the
// point rather than an oversight. Before it, a worker that needed to say "the
// approach you gave me is wrong" had to be dispatched at triage or operator —
// tiers that also hand it approve / interrupt / send_message over OTHER
// sessions. A read-only scout should not need the power to answer another
// agent's permission prompt just to tell its manager it is spending its whole
// window reading.
//
// It reaches every tier the ordinary way: `agents.reportProgress` is in
// authtoken's viewMethods, so newServer's b.allowed gate lights it up for view,
// triage and operator alike, and the tool set still derives purely from the tier
// allowlist with no hand-written exception beside it. Admitting it there does
// widen the raw bus surface — a scoped view/triage token may now CALL the method
// — and what makes that harmless is that the tool takes NO session id, in either
// direction. The caller cannot name a recipient (the host derives it from the
// caller's own parentSessionId, or refuses), and it cannot name a SENDER either:
// `callerSessionId` is stamped below from the request's own token record, and
// the hub bus DELETES the field from every untrusted caller, on the local and
// the federated dispatch path both. A scoped bus connection carries no session
// identity for the router to stamp in its place, so a phone or plugin token
// calling the method directly lands on the provider's "could not identify your
// session" refusal — the reach it gains is a method that always refuses. Only a
// session credential going through this facade can actually send anything.
//
// The BOUNDS (length, one per minute, twenty per session, no duplicates) live
// host-side in the provider, not here, and every one of them refuses out loud —
// a worker that believes it reported and did not is the failure this tool exists
// to prevent. Their messages come back to the model verbatim as the tool error.
package main

import (
	"context"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

const reportProgressMethod = "agents.reportProgress"

// sessionTokenPrefix is how the desktop labels a per-session facade token
// (authtoken Record.Label). It is the facade's only handle on WHICH session is
// calling: the bus connection underneath is one shared host-token connection.
const sessionTokenPrefix = "session:"

type reportProgressIn struct {
	Note          string `json:"note" jsonschema:"ONE line, in your own words, about your own run — a phase that landed, an approach that is not working, a budget you are burning faster than expected. Not a report: say what changed for your manager's DECISION and leave the detail for your final message, which it receives in full. Max 500 characters"`
	NeedsDecision bool   `json:"needsDecision,omitempty" jsonschema:"true if you are BLOCKED on your manager's answer rather than just keeping it informed; it is marked NEEDS A DECISION. The channel is one-way either way — your manager replies (if it replies) with a message to you, so keep working if you can"`
}

// reportProgressCall is what actually goes on the wire: the tool's own fields
// plus the caller identity the FACADE resolved. Separate from reportProgressIn
// so `callerSessionId` cannot appear in the tool's input schema — a field the
// model can see is a field the model can set, and this one is the containment.
type reportProgressCall struct {
	CallerSessionID string `json:"callerSessionId,omitempty"`
	Note            string `json:"note"`
	NeedsDecision   bool   `json:"needsDecision,omitempty"`
}

// callerSessionID reads the calling session out of the request's resolved token
// label, or "" when the credential carries no session identity (the static MCP
// token, the untokened loopback default, a plugin). Empty is forwarded as empty
// rather than guessed at: the provider owns the refusal message, so there is one
// sentence explaining it instead of two that can drift.
func callerSessionID(ctx context.Context) string {
	label := tokenLabelFrom(ctx)
	if !strings.HasPrefix(label, sessionTokenPrefix) {
		return ""
	}
	return strings.TrimSpace(strings.TrimPrefix(label, sessionTokenPrefix))
}

// addProgressTool registers report_progress on a tier's server. Called behind
// the caller's own b.allowed check, like every other tool here — see the file
// comment for why the method it forwards is admissible at the view tier.
func addProgressTool(b *build) {
	const name = "report_progress"
	const desc = "Tell the agent that dispatched you how your task is going, mid-task, in ONE line — a phase landed, the approach you were given is wrong, you are burning context faster than expected. It goes to your manager and nowhere else (you cannot name a recipient, and you have no way to reach any other session with it), it does NOT end your turn or your task, and your final message still reaches your manager in full when you finish. Limited to one per minute and 20 per session; use it when your manager's next decision would change, not to narrate."
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: reportProgressMethod, Group: b.group})

	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in reportProgressIn) (*mcp.CallToolResult, any, error) {
			// Not federated: a worker's parent is a session on the hub that
			// dispatched it, so there is no peer to route to and no `hub:` field
			// on the input to take.
			return b.forward(ctx, reportProgressMethod, reportProgressCall{
				CallerSessionID: callerSessionID(ctx),
				Note:            in.Note,
				NeedsDecision:   in.NeedsDecision,
			})
		})
}
