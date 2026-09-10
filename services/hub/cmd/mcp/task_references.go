package main

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Task references: the narrow manager-facing write on a task row.
//
// It rides the existing fleetWorkflows.request capability — same bus refusal of
// scoped/plugin/federated connections, same facade-stamped callerSessionId, same
// host-side ownership check (live local manager, its OWN task, that task's exact
// project). Deliberately NOT exposed: the host-user step waiver, any generic task
// mutation, and anything that would let an agent edit another manager's task.
//
// Semantics are additive upsert/remove on exact entries, so recording a PR the user
// pasted into chat cannot wipe the references the host user typed into the Inspector.
// The host validates URL scheme, credentials, lengths, counts and duplicates; nothing
// here fetches a URL or asks a provider whether the PR exists.
type taskRefsIn struct {
	TaskID               string           `json:"taskId" jsonschema:"the exact task id you own, from start_workflow/next_workflow_step"`
	Cwd                  string           `json:"cwd" jsonschema:"that task's project directory"`
	ExpectedTaskRevision *int             `json:"expectedTaskRevision,omitempty" jsonschema:"required CAS for updates: the taskRevision from the most recent get_task_references/update_task_references"`
	Upsert               []map[string]any `json:"upsert,omitempty" jsonschema:"entries to add or replace by exact identity: {kind:'pullRequest',number?,url?} | {kind:'ticket',id,url?} | {kind:'reference',label,url}"`
	Remove               []map[string]any `json:"remove,omitempty" jsonschema:"entries to remove by exact identity: {kind:'pullRequest'} | {kind:'ticket',id} | {kind:'reference',label}"`
}

func addTaskReferenceTools(b *build) {
	const method = "fleetWorkflows.request"
	if !b.allowed(method) {
		return
	}
	for _, item := range []struct{ name, op, desc string }{
		{"get_task_references", "taskReferences", "Read the PR/ticket/link references recorded on one of YOUR tasks, with the taskRevision to pass back as expectedTaskRevision. Desktop local only; headless returns unavailable."},
		{"update_task_references", "setTaskReferences", "Record a PR, ticket or named link the user gave you on YOUR exact task, as upsert/remove entries under expectedTaskRevision CAS. Stores the supplied id/URL as an unverified reference; never fetches it, guesses a PR number, or edits any other task field. Preserve unmentioned references; ask the user if task attribution is ambiguous. Do not scrape transcripts."},
	} {
		item := item
		b.tools = append(b.tools, toolInfo{Name: item.name, Desc: item.desc, Method: method, Group: "workflows"})
		mcp.AddTool(b.s, &mcp.Tool{Name: item.name, Description: item.desc}, func(ctx context.Context, _ *mcp.CallToolRequest, in taskRefsIn) (*mcp.CallToolResult, any, error) {
			// Session identity is request-local, never accepted as a tool argument.
			if callerSessionID(ctx) == "" {
				return nil, nil, fmt.Errorf("Task references require a local authenticated session")
			}
			if in.TaskID == "" || in.Cwd == "" {
				return nil, nil, fmt.Errorf("taskId and cwd are required and must name a task you own")
			}
			if item.op == "setTaskReferences" {
				if in.ExpectedTaskRevision == nil {
					return nil, nil, fmt.Errorf("expectedTaskRevision is required; call get_task_references first")
				}
				if len(in.Upsert) == 0 && len(in.Remove) == 0 {
					return nil, nil, fmt.Errorf("supply at least one upsert or remove entry")
				}
			}
			raw, _ := json.Marshal(in)
			var wire map[string]any
			_ = json.Unmarshal(raw, &wire)
			wire["op"] = item.op
			wire["callerSessionId"] = callerSessionID(ctx)
			return b.forward(ctx, method, wire)
		})
	}
}
