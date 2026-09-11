package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestWorkflowOperationSchemasAndCompactTransport(t *testing.T) {
	ctx := context.WithValue(context.Background(), tokenLabelKey{}, "session:manager")
	var wires []map[string]any
	cs := connectToolClient(t, ctx, func(b *build) {
		b.caller = func(_ context.Context, method string, params any) (json.RawMessage, error) {
			raw, _ := json.Marshal(params)
			var wire map[string]any
			_ = json.Unmarshal(raw, &wire)
			wires = append(wires, wire)
			return json.RawMessage(`{"ok":true,"task":{"taskId":"t","workflow":{"hash":"pinned","templates":{"ship":{"body":"REPEATED BODY","params":[{"name":"task"}],"resultSchema":{"type":"object"}}},"definition":{"id":"policy"},"steps":[{"outcome":{"verdict":"failed","exact":9007199254740993}}]}},"instructions":"next action"}`), nil
		}
		addWorkflowTools(b)
		addManagerRequestTools(b)
	})
	listed, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	schemas := map[string]map[string]any{}
	total := 0
	for _, tool := range listed.Tools {
		raw, _ := json.Marshal(tool.InputSchema)
		total += len(raw) + len(tool.Description)
		var schema map[string]any
		_ = json.Unmarshal(raw, &schema)
		schemas[tool.Name] = schema["properties"].(map[string]any)
	}
	t.Logf("workflow + inbox descriptions/schemas: %d bytes", total)
	if total > 12500 {
		t.Fatalf("schema context regression: %d bytes", total)
	}
	for _, name := range []string{"list_manager_requests", "get_manager_request", "accept_task_outcome"} {
		if _, ok := schemas[name]["intents"]; ok {
			t.Fatalf("%s pays resolution schema tax", name)
		}
	}
	for _, name := range []string{"start_workflow", "next_workflow_step", "decide_workflow_step"} {
		if _, ok := schemas[name]["definition"]; ok {
			t.Fatalf("%s pays definition edit tax", name)
		}
	}
	for _, compact := range []bool{false, true} {
		result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "next_workflow_step", Arguments: map[string]any{"taskId": "t", "cwd": "/repo", "compact": compact}})
		if err != nil || result.IsError {
			t.Fatalf("%v %v", result, err)
		}
		body := result.Content[0].(*mcp.TextContent).Text
		if strings.Contains(body, "REPEATED BODY") == compact {
			t.Fatalf("bad projection: %s", body)
		}
		for _, kept := range []string{"pinned", "resultSchema", "verdict", "failed", "next action", "policy", "9007199254740993"} {
			if !strings.Contains(body, kept) {
				t.Fatalf("lost evidence %s", kept)
			}
		}
		wire := wires[len(wires)-1]
		if _, ok := wire["compact"]; ok {
			t.Fatal("facade flag leaked to host")
		}
		if wire["callerSessionId"] != "manager" || wire["op"] != "next" {
			t.Fatal(wire)
		}
	}
	// Null means project inheritance; false is a real conditional decision.
	for _, call := range []mcp.CallToolParams{
		{Name: "select_project_workflow", Arguments: map[string]any{"cwd": "/repo", "workflowId": nil, "expectedRevision": 0}},
		{Name: "decide_workflow_step", Arguments: map[string]any{"taskId": "t", "cwd": "/repo", "stepId": "scout", "run": false, "reason": "bounded"}},
		{Name: "clone_workflow", Arguments: map[string]any{"id": "policy", "expectedRevision": 1}},
		{Name: "list_manager_requests", Arguments: map[string]any{"view": "pending"}},
	} {
		result, err := cs.CallTool(ctx, &call)
		if err != nil || result.IsError {
			t.Fatalf("%s: %v %v", call.Name, result, err)
		}
	}
	if value, exists := wires[len(wires)-4]["workflowId"]; !exists || value != nil {
		t.Fatal("lost inheritance null")
	}
	if wires[len(wires)-3]["run"] != false {
		t.Fatal("lost false decision")
	}
	if wires[len(wires)-1]["view"] != "pending" {
		t.Fatal("lost pending view")
	}
	before := len(wires)
	result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "next_workflow_step", Arguments: map[string]any{"taskId": "t", "cwd": "/repo", "callerSessionId": "foreign"}})
	if err != nil || !result.IsError || len(wires) != before {
		t.Fatal("spoofed identity reached host")
	}
}
