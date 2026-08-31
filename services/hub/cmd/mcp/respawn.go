// respawn_with — kill a scope-creeping worker, redispatch it surgically with
// the diagnosis baked in.
//
// The user's standing rule for a worker that has wandered is to stop it and
// redispatch, narrowly, with what went wrong stated up front. Today that means
// re-authoring the whole first message by hand: reading the original dispatch
// back out of the transcript, retyping it, and remembering the cwd, model,
// provider, effort, label and parent it was launched with. The retyping is
// where the original task quietly changes.
//
// This is a FACADE-side composition, not a new capability: it reads the
// original session (sessions.snapshot + sessions.conversation) and then calls
// agents.spawn and agents.sendMessage — three methods the caller's tier already
// holds. Nothing new is registered on the bus, so there is no new hub method to
// classify, no new provider, and no new door.
//
// AND IT GOES THROUGH THE SAME SPAWN GATE. The composed spawn is handed to
// spawnWithGrants (main.go), the exact function spawn_agent's own handler
// calls, so the profile-dispatch grant, the config-default resolution and the
// full-access clamp all apply identically. That matters specifically here: this
// tool reads the ORIGINAL session's permission mode, and a hand-rolled second
// copy of the forward path would have turned "clone this worker" into a way to
// re-request a bypass without the grant check. The clone of a bypassed worker
// is bypassed only if the caller's own token still carries the grant.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// respawnWithIn is respawn_with's input. Everything except sessionId and
// amendment is an OVERRIDE of what the original session recorded.
type respawnWithIn struct {
	hubArg
	SessionID  string `json:"sessionId" jsonschema:"the session to clone — usually one you have just stopped"`
	Amendment  string `json:"amendment" jsonschema:"the correction, in your words: what went wrong and what the successor must do differently. It is appended to the original task under a clear heading, so state the DIAGNOSIS, not the whole task again"`
	Label      string `json:"label,omitempty" jsonschema:"label for the successor; defaults to the original's with a retry marker"`
	Model      string `json:"model,omitempty" jsonschema:"override the model (e.g. step up for a task that proved harder than it looked); defaults to the original's"`
	Effort     string `json:"effort,omitempty" jsonschema:"override the reasoning effort; defaults to the original's"`
	Cwd        string `json:"cwd,omitempty" jsonschema:"override the working directory; defaults to the original's — which for a ship task is its WORKTREE, so the successor continues on the same branch with the partial work in place. Pass the repo path (with worktree:true) to start clean instead"`
	Role       string `json:"role,omitempty" jsonschema:"override the work ROLE the successor is dispatched as (scout | implementer | reviewer | deep_reviewer | fixer | complex_fixer | validator | diagnostician | mechanical | judge); defaults to the original's. Set it when the redispatch changes what the worker IS — a failed implementer respawned to diagnose is a diagnostician, not an implementer"`
	Capability string `json:"capability,omitempty" jsonschema:"override the model CAPABILITY the successor is dispatched at (cheap | balanced | frontier | frontier_max | reviewer | deep_reviewer | frontier_plus); defaults to the original's. Copy it from a fresh select_model answer rather than raising it by hand — the host still clamps it to the target directory's ceiling"`
	ToolScope  string `json:"toolScope,omitempty" jsonschema:"facade tier for the successor (view/triage/operator). NOT inherited — a session's snapshot does not record it — so restate it if the original had one"`
	Worktree   bool   `json:"worktree,omitempty" jsonschema:"carve a FRESH isolated worktree for the successor instead of reusing the original's cwd; use when the partial work should be abandoned rather than continued"`
}

func (in *respawnWithIn) takeHubField() string { return in.takeHub() }

// respawnSnapshot is the subset of a session snapshot a respawn needs. Every
// field is optional: a provider shape that omits one degrades to "not
// inherited", never to an error — a respawn that refuses because a snapshot
// lacked `effort` would be worse than one that starts at the default.
type respawnSnapshot struct {
	Cwd             string `json:"cwd"`
	Label           string `json:"label"`
	Provider        string `json:"provider"`
	Transport       string `json:"transport"`
	ParentSessionID string `json:"parentSessionId"`
	Settings        struct {
		Model          string `json:"model"`
		Effort         string `json:"effort"`
		PermissionMode string `json:"permissionMode"`
	} `json:"settings"`
	LivePermissionMode string `json:"livePermissionMode"`
	// ResultSchema is the structured-result contract the original was spawned
	// with (spawn_agent's resultSchema, recorded on the snapshot by
	// setSpawnMeta). Without inheriting it, "stop it, respawn with the
	// diagnosis baked in" silently downgrades a structured dispatch to a prose
	// one — the caller gets prose back where it expected a validated object,
	// with nothing announcing the downgrade.
	ResultSchema map[string]any `json:"resultSchema"`
	// Routing is what the ROUTING layer said this worker is, recorded against
	// the session at spawn time and surfaced back on the snapshot (the brain's
	// enrichSnapshot overlay, from spawnMeta.Role/Capability/DecisionID).
	//
	// Inherited for the same reason resultSchema is, and one reason more.
	// `role` is not a label: `fresh` enforcement in the routing layer keys off
	// the DECLARED role, so a reviewer that loses its role on respawn loses its
	// freshness guarantee with nothing announcing the loss. And a spawn that
	// declares neither field gets no routing decision at all, so dropping them
	// quietly demotes a routed dispatch to an unrouted one.
	//
	// `decisionId` is deliberately NOT inherited: it joins a worker to the
	// select_model answer that produced it, and the successor was produced by
	// the dispatcher's judgement about a FAILURE, not by that decision. Copying
	// it would file the redispatch under a decision that never asked for it.
	Routing struct {
		Role       string `json:"role"`
		Capability string `json:"capability"`
	} `json:"routing"`
}

// RESPAWN_HEADING separates the inherited task from the correction. Spelled
// loudly on purpose: the successor is reading a message whose first half was
// written for a DIFFERENT agent, and it must not mistake the amendment for a
// footnote.
const respawnHeading = "\n\n--- CORRECTION FROM YOUR DISPATCHER (this supersedes anything above it that conflicts) ---\n\n"

// firstUserMessage pulls the original DISPATCH out of a conversation snapshot:
// the first user_message item's text. That is the task as it was actually
// given, which is exactly what hand-retyping puts at risk.
func firstUserMessage(raw json.RawMessage) string {
	var snap conversationSnap
	if json.Unmarshal(raw, &snap) != nil {
		return ""
	}
	for _, item := range snap.Items {
		if itemKind(item) != "user_message" {
			continue
		}
		var m struct {
			Text string `json:"text"`
		}
		if json.Unmarshal(item, &m) == nil && strings.TrimSpace(m.Text) != "" {
			return m.Text
		}
	}
	return ""
}

// addRespawnTool registers respawn_with. It is admitted only when the tier may
// call every method it composes — spawn, snapshot, conversation and
// sendMessage. Deriving the gate from the parts rather than asserting a tier is
// the same rule addTool follows, and it means respawn_with can never be reachable
// where spawn_agent is not.
func addRespawnTool(b *build) {
	const (
		spawnMethod = "agents.spawn"
		snapMethod  = "sessions.snapshot"
		convMethod  = "sessions.conversation"
		sendMethod  = "agents.sendMessage"
	)
	for _, m := range []string{spawnMethod, snapMethod, convMethod, sendMethod} {
		if !b.allowed(m) {
			return
		}
	}
	const name = "respawn_with"
	const desc = "Redispatch a worker: clone a session's ORIGINAL task and cwd/model/provider/parent/role, append your correction, and start a fresh agent with both. The standing move for a worker that has crept out of scope — stop it (signal SIGTERM, then close_session), then respawn_with the diagnosis baked in, instead of re-authoring the whole first message by hand. Returns the new sessionId."
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: spawnMethod, Group: b.group})

	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in respawnWithIn) (*mcp.CallToolResult, any, error) {
			if strings.TrimSpace(in.SessionID) == "" {
				return toolError("respawn_with requires sessionId")
			}
			// An empty amendment makes this a plain clone, which is a worker
			// that will do the same thing again. Refused out loud: the whole
			// point is redispatching SURGICALLY.
			if strings.TrimSpace(in.Amendment) == "" {
				return toolError("respawn_with requires an amendment — what the successor must do differently. Cloning a scope-creeping worker with no correction just repeats it; use spawn_agent for a plain new dispatch.")
			}

			peer := in.takeHubField()
			route := func(m string) string {
				if peer == "" {
					return m
				}
				return "hub:" + peer + "/" + m
			}

			rawSnap, err := b.call(ctx, route(snapMethod), map[string]string{"sessionId": in.SessionID})
			if err != nil {
				return toolError(fmt.Sprintf("respawn_with: could not read session %s: %v", in.SessionID, err))
			}
			var snap respawnSnapshot
			if json.Unmarshal(rawSnap, &snap) != nil {
				return toolError(fmt.Sprintf("respawn_with: session %s returned a snapshot this tool could not read", in.SessionID))
			}

			rawConv, err := b.call(ctx, route(convMethod), map[string]string{"sessionId": in.SessionID})
			if err != nil {
				return toolError(fmt.Sprintf("respawn_with: could not read %s's conversation: %v", in.SessionID, err))
			}
			task := firstUserMessage(rawConv)
			if strings.TrimSpace(task) == "" {
				// No original task means there is nothing to clone, and a
				// successor launched with the amendment alone would be a
				// dispatch the caller never wrote. Refuse rather than invent.
				return toolError(fmt.Sprintf("respawn_with: %s has no first user message to clone — it was never given a task. Use spawn_agent to dispatch fresh.", in.SessionID))
			}

			spawn := spawnAgentIn{
				Hub:             peer,
				Provider:        snap.Provider,
				Transport:       snap.Transport,
				Cwd:             firstNonEmpty(in.Cwd, snap.Cwd),
				Model:           firstNonEmpty(in.Model, snap.Settings.Model),
				Effort:          firstNonEmpty(in.Effort, snap.Settings.Effort),
				Label:           firstNonEmpty(in.Label, retryLabel(snap.Label)),
				ParentSessionId: snap.ParentSessionID,
				ToolScope:       in.ToolScope,
				Worktree:        in.Worktree,
				ResultSchema:    snap.ResultSchema,
				Role:            firstNonEmpty(in.Role, snap.Routing.Role),
				Capability:      firstNonEmpty(in.Capability, snap.Routing.Capability),
				// The composed dispatch rides the SPAWN now, rather than a
				// follow-up sendMessage below.
				//
				// On whether respawn_with should INHERIT a first message the
				// way it inherits resultSchema: no, and the distinction is
				// exact. resultSchema had to be inherited because it is a
				// property of the DISPATCH CONTRACT recorded nowhere else, so
				// dropping it silently downgraded a structured dispatch to a
				// prose one. A first message needs no inheriting because this
				// tool already recovers something strictly better:
				// `firstUserMessage` reads the task out of the original's
				// CONVERSATION — the text the agent actually received — where
				// the spawn param would only be a staler second copy of the
				// same thing, plus a rule about which one wins.
				//
				// Using the field to DELIVER is a plain improvement though: it
				// closes the window this tool's own error path documents
				// ("spawned session:%s but could not deliver the task"), and
				// spawnWithGrants still falls back to sendMessage when the
				// provider does not confirm it took the prompt.
				Message: task + respawnHeading + in.Amendment,
			}
			// The original's permission mode is REQUESTED, not granted: it goes
			// through spawnWithGrants exactly as a hand-typed skipPermissions
			// would, so an ungranted caller's clone starts with approvals on
			// even if the original ran bypassed. Live mode wins over the
			// spawn-time one — it is what the worker was actually running at.
			//
			// Always EXPLICIT, both ways. Leaving the field nil for a
			// non-bypassed original would hand it to spawnWithGrants' omitted
			// path, where a granted caller's full-access grant (or the config
			// default) would resolve it to true — silently upgrading a worker
			// that deliberately ran with approvals ON. respawn_with clones the
			// original; it does not re-decide this.
			bypassed := permissionModeMeansBypass(
				firstNonEmpty(snap.LivePermissionMode, snap.Settings.PermissionMode))
			spawn.SkipPermissions = &bypassed

			res, _, err := spawnWithGrants(ctx, b, spawnMethod, spawn)
			if err != nil {
				return nil, nil, err
			}
			if res.IsError {
				return res, nil, nil
			}
			newID := sessionIDFrom(res)
			if newID == "" {
				// The spawn succeeded but we cannot address the successor, so
				// the task cannot be delivered. Say so instead of reporting a
				// respawn that never received its dispatch.
				return toolError("respawn_with: the spawn returned no sessionId, so the composed task could not be delivered. The new agent (if any) is idle — find it with list_agents and send it the task yourself. Spawn result: " + resultText(res))
			}

			// Delivery already happened inside spawnWithGrants — with the
			// spawn when the provider confirmed it took the prompt, by a
			// fallback sendMessage when it did not. An undelivered task comes
			// back as res.IsError above, so there is nothing left to send. Two
			// sends here would be worse than none: the successor would read the
			// whole dispatch twice.

			out, merr := json.Marshal(map[string]any{
				"sessionId":  newID,
				"clonedFrom": in.SessionID,
				"cwd":        spawn.Cwd,
				"label":      spawn.Label,
				"role":       spawn.Role,
				"capability": spawn.Capability,
				"note":       "The successor was sent the original task plus your correction. Its permission mode was re-judged by the same grant check a fresh spawn_agent gets — it is not inherited.",
			})
			if merr != nil {
				return res, nil, nil
			}
			return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: string(out)}}}, nil, nil
		})
}

// retryLabel marks a successor so the sidebar does not show two identically
// named cards. An unlabelled original stays unlabelled — inventing a name is
// the fleet-manager UI's job, not this tool's.
func retryLabel(label string) string {
	if strings.TrimSpace(label) == "" {
		return ""
	}
	return label + " (redispatch)"
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if strings.TrimSpace(v) != "" {
			return v
		}
	}
	return ""
}

// toolError renders a facade-local refusal in the same shape forward() uses for
// a hub error, so a caller cannot tell (or need to tell) where it came from.
func toolError(msg string) (*mcp.CallToolResult, any, error) {
	return &mcp.CallToolResult{
		IsError: true,
		Content: []mcp.Content{&mcp.TextContent{Text: msg}},
	}, nil, nil
}

// resultText flattens a tool result's text content (for embedding a spawn's own
// answer in a diagnostic).
func resultText(res *mcp.CallToolResult) string {
	var b strings.Builder
	for _, c := range res.Content {
		if t, ok := c.(*mcp.TextContent); ok {
			b.WriteString(t.Text)
		}
	}
	return b.String()
}

// sessionIDFrom digs the new session id out of a spawn result. agents.spawn
// answers `{ "sessionId": "…" }` on both providers; the bare-string form is
// tolerated because forward() normalizes some results to plain text.
func sessionIDFrom(res *mcp.CallToolResult) string {
	text := strings.TrimSpace(resultText(res))
	if text == "" || text == "ok" {
		return ""
	}
	var obj struct {
		SessionID string `json:"sessionId"`
	}
	if json.Unmarshal([]byte(text), &obj) == nil && obj.SessionID != "" {
		return obj.SessionID
	}
	var bare string
	if json.Unmarshal([]byte(text), &bare) == nil && bare != "" {
		return bare
	}
	return ""
}
