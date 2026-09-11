package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type managerContextTask struct {
	TaskID string `json:"taskId"`
	Cwd    string `json:"cwd" jsonschema:"the task's exact local project directory"`
}
type managerContextIn struct {
	Tasks []managerContextTask `json:"tasks,omitempty" jsonschema:"up to 4 tasks to inspect alongside your pending inbox; use task IDs from the current wake/request, not the whole fleet"`
}

// One read turn for inbox + relevant tasks. Each task is checked by the normal
// host ownership gate. Full template/history bodies stay out of the answer.
func addManagerContextTool(b *build) {
	const method = "fleetWorkflows.request"
	if !b.allowed(method) {
		return
	}
	const name = "manager_context"
	const desc = "Read your pending inbox and up to four owned task states/next dispatch inputs in one call. Compact evidence and partial errors; no mutations or polling."
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: method, Group: "workflows"})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc}, func(ctx context.Context, _ *mcp.CallToolRequest, in managerContextIn) (*mcp.CallToolResult, any, error) {
		caller := callerSessionID(ctx)
		if caller == "" {
			return toolError("manager_context requires an authenticated local manager")
		}
		if len(in.Tasks) > 4 {
			return toolError("manager_context accepts at most four tasks")
		}
		seen := map[string]bool{}
		for _, task := range in.Tasks {
			if strings.TrimSpace(task.TaskID) == "" || strings.TrimSpace(task.Cwd) == "" || seen[task.TaskID] {
				return toolError("tasks require unique taskId values and cwd")
			}
			seen[task.TaskID] = true
		}
		results := make([]json.RawMessage, len(in.Tasks)+1)
		var wg sync.WaitGroup
		for i := range results {
			wg.Add(1)
			go func(i int) {
				defer wg.Done()
				wire := map[string]any{"op": "requestInbox", "view": "pending", "callerSessionId": caller}
				if i > 0 {
					wire["op"] = "next"
					delete(wire, "view")
					wire["taskId"] = in.Tasks[i-1].TaskID
					wire["cwd"] = in.Tasks[i-1].Cwd
				}
				raw, err := b.call(ctx, method, wire)
				if err != nil {
					raw, _ = json.Marshal(map[string]any{"ok": false, "error": err.Error()})
				}
				if !json.Valid(raw) {
					raw, _ = json.Marshal(map[string]any{"ok": false, "error": "Host returned an unreadable context response"})
				}
				if i > 0 {
					raw = managerTaskContext(raw, in.Tasks[i-1])
				}
				results[i] = raw
			}(i)
		}
		wg.Wait()
		return fleetValueResult(map[string]any{"inbox": results[0], "tasks": results[1:]}, false)
	})
}

func managerTaskContext(raw json.RawMessage, request managerContextTask) json.RawMessage {
	fail := func(message string) json.RawMessage {
		out, _ := json.Marshal(map[string]any{"ok": false, "taskId": request.TaskID, "cwd": request.Cwd, "error": message})
		return out
	}
	var response map[string]json.RawMessage
	if json.Unmarshal(raw, &response) != nil || response == nil {
		return fail("Unreadable task context")
	}
	if string(response["ok"]) != "true" {
		response["taskId"], _ = json.Marshal(request.TaskID)
		response["cwd"], _ = json.Marshal(request.Cwd)
		out, _ := json.Marshal(response)
		return out
	}
	var task map[string]json.RawMessage
	if json.Unmarshal(response["task"], &task) != nil || len(task) == 0 {
		return fail("Missing task context")
	}
	var id string
	if json.Unmarshal(task["taskId"], &id) != nil || id != request.TaskID {
		return fail("Mismatched task context")
	}
	result := map[string]any{"ok": true, "taskId": request.TaskID, "cwd": request.Cwd}
	for _, key := range []string{"title", "revision", "cancelled", "dependsOn", "acceptedOutcome", "links"} {
		if value, ok := task[key]; ok {
			result[key] = value
		}
	}
	var workflow map[string]json.RawMessage
	if json.Unmarshal(task["workflow"], &workflow) == nil {
		for _, key := range []string{"steps", "hash"} {
			if value, ok := workflow[key]; ok {
				result[key] = value
			}
		}
	}
	for _, key := range []string{"instructions", "dispatch"} {
		if value, ok := response[key]; ok {
			result[key] = value
		}
	}
	out, err := json.Marshal(result)
	if err != nil {
		return raw
	}
	// An unusually large result stays available through next_workflow_step;
	// never silently trim evidence or pretend that a deferred result was read.
	if len(out) > 24*1024 {
		out, _ = json.Marshal(map[string]any{"ok": true, "taskId": request.TaskID, "cwd": request.Cwd, "contentDeferred": true, "bytes": len(out), "instructions": fmt.Sprintf("Read next_workflow_step for task %s with compact:true; this task's evidence exceeds the context batch budget.", request.TaskID)})
	}
	return out
}
