package main

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestManagerRequestIdentityAndScope(t *testing.T) {
	for _, name := range []string{"list_manager_requests", "get_manager_request", "resolve_manager_request", "accept_task_outcome"} {
		if !listToolsFor(t, authtoken.ScopeOperator)[name] || listToolsFor(t, authtoken.ScopeView)[name] || listToolsFor(t, authtoken.ScopeTriage)[name] {
			t.Fatalf("bad inbox tool scope: %s", name)
		}
	}
	ctx := context.WithValue(context.Background(), tokenLabelKey{}, "session:manager")
	var got map[string]any
	cs := connectToolClient(t, ctx, func(b *build) {
		b.caller = func(_ context.Context, method string, params any) (json.RawMessage, error) {
			if method != "fleetWorkflows.request" {
				t.Fatal(method)
			}
			raw, _ := json.Marshal(params)
			_ = json.Unmarshal(raw, &got)
			return json.RawMessage(`{"ok":false,"code":"conflict","request":{"host":{"requestId":"r","revision":3}}}`), nil
		}
		addManagerRequestTools(b)
	})
	result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "resolve_manager_request", Arguments: map[string]any{
		"requestId": "r", "expectedRevision": 2, "intents": []any{map[string]any{"key": "status", "kind": "none", "reason": "status question"}},
	}})
	if err != nil || result.IsError || got["callerSessionId"] != "manager" || got["op"] != "resolveRequest" {
		t.Fatalf("bad request transport: %v %v %v", result, err, got)
	}
	got = nil
	result, err = cs.CallTool(ctx, &mcp.CallToolParams{Name: "get_manager_request", Arguments: map[string]any{"requestId": "r", "callerSessionId": "foreign"}})
	if err != nil || !result.IsError || got != nil {
		t.Fatalf("spoof reached host: %v %v %v", result, err, got)
	}
}
