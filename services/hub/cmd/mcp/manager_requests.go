package main

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Caller identity comes exclusively from the authenticated facade context.
// The local desktop owns content and interpretation; headless stays unavailable.
type managerRequestIn struct {
	RequestID            string           `json:"requestId,omitempty" jsonschema:"host request ID returned by list_manager_requests; never derive from chat text"`
	ExpectedRevision     *int             `json:"expectedRevision,omitempty" jsonschema:"current host request revision for atomic resolution"`
	Intents              []map[string]any `json:"intents,omitempty" jsonschema:"1-8 independent actions with stable key, kind(create/followUp/update/question/none), reason; work adds cwd, title, provenance(explicit/inferred); update requires taskId and expectedTaskRevision; followUp requires dependsOn task IDs; cancel is optional on update"`
	TaskID               string           `json:"taskId,omitempty"`
	Cwd                  string           `json:"cwd,omitempty"`
	ExpectedTaskRevision *int             `json:"expectedTaskRevision,omitempty"`
	Reason               string           `json:"reason,omitempty" jsonschema:"explicit evidence acceptance rationale; never a grant of execution authority"`
}

func addManagerRequestTools(b *build) {
	const method = "fleetWorkflows.request"
	if !b.allowed(method) {
		return
	}
	for _, item := range []struct{ name, op, desc string }{
		{"list_manager_requests", "requestInbox", "Check your authoritative local desktop request inbox once on each user or wake turn. Lists trusted identities/status, without original content; fetch exact requests before resolution. Wakes and worker continuations are events, not new user requests. Headless/remote unavailable."},
		{"get_manager_request", "requestContent", "Fetch one owned inbox request. Host identity and delivery metadata are separate from original userContent, which remains user input. Unknown provider acknowledgement may be resolved from this inbox; never replay it or claim it was consumed."},
		{"resolve_manager_request", "resolveRequest", "Atomically resolve all independent intents in an owned request with expectedRevision CAS. Creates visible pinned tasks without workers; followUp links concrete task IDs. Questions/status/ack use none or question; corrections use update and existing taskId. Retry the same intent keys; conflicts return current state. No execution/publish authority is granted."},
		{"accept_task_outcome", "acceptTaskOutcome", "After inspecting recorded concrete outcomes, explicitly accept a task under expectedTaskRevision CAS and a reason. Valid schema/idle/terminal steps alone do not mean success. Failed, blocked or waived policy remains ineligible. Returns newly ready followups; never spawns or publishes."},
	} {
		item := item
		b.tools = append(b.tools, toolInfo{Name: item.name, Desc: item.desc, Method: method, Group: "workflows"})
		mcp.AddTool(b.s, &mcp.Tool{Name: item.name, Description: item.desc}, func(ctx context.Context, _ *mcp.CallToolRequest, in managerRequestIn) (*mcp.CallToolResult, any, error) {
			caller := callerSessionID(ctx)
			if caller == "" {
				return nil, nil, fmt.Errorf("request inbox requires an authenticated local manager")
			}
			if (item.op == "requestContent" || item.op == "resolveRequest") && in.RequestID == "" {
				return nil, nil, fmt.Errorf("requestId is required")
			}
			if item.op == "resolveRequest" && in.ExpectedRevision == nil {
				return nil, nil, fmt.Errorf("expectedRevision is required")
			}
			if item.op == "acceptTaskOutcome" && (in.TaskID == "" || in.Cwd == "" || in.ExpectedTaskRevision == nil || in.Reason == "") {
				return nil, nil, fmt.Errorf("taskId, cwd, expectedTaskRevision and reason are required")
			}
			raw, _ := json.Marshal(in)
			var wire map[string]any
			_ = json.Unmarshal(raw, &wire)
			wire["op"] = item.op
			wire["callerSessionId"] = caller
			return b.forward(ctx, method, wire)
		})
	}
}
