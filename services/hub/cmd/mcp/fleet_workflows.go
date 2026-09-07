package main

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type workflowIn struct {
	ID               string         `json:"id,omitempty" jsonschema:"workflow definition id"`
	ExpectedRevision *int           `json:"expectedRevision,omitempty" jsonschema:"required CAS revision for edits/clones/deletes; selectionRevision for selection"`
	Definition       map[string]any `json:"definition,omitempty" jsonschema:"strict Fleet definition: id/revision/name/description/enabled/steps only; no privileges or scripts"`
	Name             string         `json:"name,omitempty"`
	Cwd              string         `json:"cwd,omitempty" jsonschema:"local project directory; omit for global selection"`
	WorkflowID       *string        `json:"workflowId,omitempty" jsonschema:"enabled workflow id for selection; null means project inherits global"`
	TaskID           string         `json:"taskId,omitempty"`
	Title            string         `json:"title,omitempty"`
	StepID           string         `json:"stepId,omitempty"`
	Run              *bool          `json:"run,omitempty"`
	Reason           string         `json:"reason,omitempty"`
}

func addWorkflowTools(b *build) {
	const method = "fleetWorkflows.request"
	if !b.allowed(method) {
		return
	}
	for _, item := range []struct{ name, op, desc string }{
		{"list_workflows", "list", "List local desktop Fleet workflows, revisioned selections and dispatch template inputs. Headless/older peers unavailable."},
		{"get_workflow", "get", "Read a Fleet workflow definition."},
		{"validate_workflow", "validate", "Validate a declarative 1–8 step definition; never grants permissions or launches agents."},
		{"create_workflow", "create", "Create a custom Fleet definition with a unique id, revision 1."},
		{"update_workflow", "update", "Replace a custom definition using expectedRevision CAS. Active tasks remain pinned. Clone immutable starters."},
		{"clone_workflow", "clone", "Clone a definition at expectedRevision into a new custom id."},
		{"disable_workflow", "disable", "Disable an unselected custom definition at expectedRevision. Pinned tasks continue."},
		{"delete_workflow", "delete", "Delete an unselected custom definition at expectedRevision; pinned tasks retain their snapshot."},
		{"select_default_workflow", "select", "Select global default with workflowId and expectedRevision=selectionRevision; omit cwd."},
		{"select_project_workflow", "select", "Select project workflow with cwd and expectedRevision=selectionRevision; workflowId:null inherits."},
		{"start_workflow", "start", "BEFORE the first worker for each new project task: resolve selected workflow and freeze its definition/templates/result contracts. Requires cwd/title and your authenticated local manager session. Returns taskId and next instructions; never launches automatically."},
		{"next_workflow_step", "next", "Read pinned task policy and the next eligible step. Requires taskId/cwd under your manager. Follow the returned select_model/spawn_agent metadata; never infer pass from a valid result or idle."},
		{"decide_workflow_step", "decide", "Record run=true/false and a reason for the next conditional or explicit bounded repair step. Required review cannot be skipped. Requires taskId/cwd/stepId."},
	} {
		item := item
		b.tools = append(b.tools, toolInfo{Name: item.name, Desc: item.desc, Method: method, Group: "workflows"})
		mcp.AddTool(b.s, &mcp.Tool{Name: item.name, Description: item.desc}, func(ctx context.Context, _ *mcp.CallToolRequest, in workflowIn) (*mcp.CallToolResult, any, error) {
			// Session identity is request-local, never accepted as a tool argument. No Hub parameter.
			if callerSessionID(ctx) == "" {
				return nil, nil, fmt.Errorf("Fleet workflows require a local authenticated session; use desktop Settings for host management")
			}
			if item.name == "select_default_workflow" && in.Cwd != "" {
				return nil, nil, fmt.Errorf("global selection must omit cwd")
			}
			if item.name == "select_project_workflow" && in.Cwd == "" {
				return nil, nil, fmt.Errorf("project selection requires cwd")
			}
			raw, _ := json.Marshal(in)
			var wire map[string]any
			_ = json.Unmarshal(raw, &wire)
			if item.op == "select" && in.WorkflowID == nil {
				wire["workflowId"] = nil
			}
			wire["op"] = item.op
			wire["callerSessionId"] = callerSessionID(ctx)
			return b.forward(ctx, method, wire)
		})
	}
}
