package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestIntentToolScopes(t *testing.T) {
	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator} {
		listed := listToolsFor(t, scope)
		for _, name := range []string{"create_intent", "list_intents", "get_intent", "update_intent", "add_intent_context", "list_jira_connections", "create_intent_from_jira"} {
			if listed[name] != (scope == authtoken.ScopeOperator) {
				t.Errorf("%s tool %s scope mismatch", scope, name)
			}
		}
	}
}

func TestIntentToolsRouteAndPreserveRequirements(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	server := bus.NewServer(broker.New())
	server.SetToken("intent-test-owner")
	var mu sync.Mutex
	var writes []map[string]any
	workspace := map[string]any{"id": "intent-1", "projectRoot": "/project", "title": "Export", "outcome": "CSV", "constraints": "Keep permissions", "successCriteria": "Filtered rows only", "sourceUrl": "", "status": "draft", "revision": float64(2), "updatedAt": "now"}
	server.RegisterLocal(intentMethod, func(raw json.RawMessage) (any, error) {
		var input struct {
			Request map[string]any `json:"request"`
		}
		if err := json.Unmarshal(raw, &input); err != nil {
			return nil, err
		}
		mu.Lock()
		defer mu.Unlock()
		switch input.Request["action"] {
		case "list":
			return map[string]any{"action": "list", "workspaces": []any{workspace}}, nil
		case "sources":
			return map[string]any{"action": "sources", "sources": []any{}}, nil
		default:
			writes = append(writes, input.Request)
			return map[string]any{"action": input.Request["action"], "workspace": workspace}, nil
		}
	})
	hub := httptest.NewServer(server.Handler())
	defer hub.Close()
	client := busclient.New(strings.Replace(hub.URL, "http", "ws", 1)+"/bus", "intent-test-owner")
	go client.Run(ctx)
	cs := connectTo(t, ctx, newServer(client, authtoken.ScopeOperator))
	call := func(name string, args map[string]any) *mcp.CallToolResult {
		t.Helper()
		r, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: name, Arguments: args})
		if err != nil {
			t.Fatal(err)
		}
		return r
	}
	result := call("create_intent", map[string]any{"projectRoot": "/project", "title": "Export", "outcome": "CSV"})
	if result.IsError {
		t.Fatal(result.Content[0].(*mcp.TextContent).Text)
	}
	mu.Lock()
	created := writes[0]["fields"].(map[string]any)
	mu.Unlock()
	if created["status"] != "draft" || created["constraints"] != "" {
		t.Fatalf("create defaults: %#v", created)
	}
	result = call("get_intent", map[string]any{"id": "intent-1"})
	if result.IsError || !strings.Contains(result.Content[0].(*mcp.TextContent).Text, "Keep permissions") {
		t.Fatal(result.Content[0].(*mcp.TextContent).Text)
	}
	result = call("list_intents", map[string]any{"projectRoot": "/other"})
	if result.IsError || result.Content[0].(*mcp.TextContent).Text != `{"intents":[]}` {
		t.Fatal(result.Content[0].(*mcp.TextContent).Text)
	}
	result = call("update_intent", map[string]any{"id": "intent-1", "expectedRevision": 2, "expectedUpdatedAt": "now", "outcome": "CSV and JSON", "constraints": "", "reason": "User clarified formats"})
	if result.IsError {
		t.Fatal(result.Content[0].(*mcp.TextContent).Text)
	}
	mu.Lock()
	updated := writes[1]
	mu.Unlock()
	fields := updated["fields"].(map[string]any)
	if fields["title"] != "Export" || fields["successCriteria"] != "Filtered rows only" || fields["status"] != "draft" || fields["constraints"] != "" || fields["outcome"] != "CSV and JSON" || updated["expectedUpdatedAt"] != "now" {
		t.Fatalf("patch lost fields or guards: %#v", updated)
	}
	for _, args := range []map[string]any{
		{"id": "intent-1", "expectedRevision": 1, "expectedUpdatedAt": "now", "reason": "stale"},
		{"id": "intent-1", "expectedRevision": 2, "expectedUpdatedAt": "old", "reason": "stale"},
	} {
		if !call("update_intent", args).IsError {
			t.Fatal("stale edit accepted")
		}
	}
	if !call("get_intent", map[string]any{"id": "missing"}).IsError {
		t.Fatal("missing intent accepted")
	}
	result = call("add_intent_context", map[string]any{"id": "intent-1", "sourceId": "note-1", "expectedRevision": 2, "title": "Code research", "content": "See src/export.ts; open question: delimiter?"})
	if result.IsError {
		t.Fatal(result.Content[0].(*mcp.TextContent).Text)
	}
	result = call("create_intent_from_jira", map[string]any{"projectRoot": "/project", "operationId": "import-1", "integrationId": "jira-1", "expectedIntegrationVersion": 1, "identifier": "TEAM-123"})
	if result.IsError {
		t.Fatal(result.Content[0].(*mcp.TextContent).Text)
	}
	mu.Lock()
	defer mu.Unlock()
	if writes[3]["action"] != "importJiraIntent" || writes[3]["operationId"] != "import-1" || writes[3]["identifier"] != "TEAM-123" {
		t.Fatalf("wrong Jira import: %#v", writes[3])
	}
	if len(writes) != 4 {
		t.Fatalf("unexpected mutations: %#v", writes)
	}
	source := writes[2]
	connection := source["connection"].(map[string]any)
	if source["action"] != "addSource" || source["sourceId"] != "note-1" || connection["provider"] != "manual" || connection["url"] != "" || connection["credentialEnv"] != "" {
		t.Fatalf("wrong source: %#v", source)
	}
}
