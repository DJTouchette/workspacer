package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// answeringProvider registers methods on the bus and answers each call with
// the canned result from answers (falling back to an echo like fakeProvider).
func answeringProvider(t *testing.T, ctx context.Context, busURL string, answers map[string]any) {
	t.Helper()
	methods := make([]string, 0, len(answers))
	for m := range answers {
		methods = append(methods, m)
	}
	conn, _, err := websocket.Dial(ctx, busURL, nil)
	if err != nil {
		t.Fatalf("provider dial: %v", err)
	}
	reg, _ := json.Marshal(busFrame{Op: "register", Methods: methods})
	if err := conn.Write(ctx, websocket.MessageText, reg); err != nil {
		t.Fatalf("provider register: %v", err)
	}
	go func() {
		defer conn.CloseNow()
		for {
			_, data, err := conn.Read(ctx)
			if err != nil {
				return
			}
			var f busFrame
			if json.Unmarshal(data, &f) != nil || f.Op != "call" {
				continue
			}
			// A canned answer is returned VERBATIM (the catalog poller expects
			// the []pluginTools shape raw); a nil entry echoes method+params
			// like fakeProvider, for asserting routing.
			var out json.RawMessage
			if a := answers[f.Method]; a != nil {
				out, _ = json.Marshal(a)
			} else {
				out, _ = json.Marshal(map[string]any{
					"method": f.Method,
					"params": json.RawMessage(nonNil(f.Params)),
				})
			}
			reply, _ := json.Marshal(busFrame{Op: "result", ID: f.ID, Result: out})
			_ = conn.Write(ctx, websocket.MessageText, reply)
		}
	}()
}

// connectTo opens an in-memory MCP client session against a server.
func connectTo(t *testing.T, ctx context.Context, server *mcp.Server) *mcp.ClientSession {
	t.Helper()
	cT, sT := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	t.Cleanup(func() { cs.Close() })
	return cs
}

// TestPluginToolBridge proves the whole plugin-tool chain: the hub-side
// catalog method feeds the facade's poller, a token's plugin grant surfaces
// the tool on that token's server (and only that one), and calling the tool
// forwards to the plugin's bus method.
func TestPluginToolBridge(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer hub.Close()
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"

	catalogJSON := []map[string]any{{
		"pluginId": "djtouchette.jira",
		"tools": []map[string]any{{
			"name":        "search",
			"description": "Search Jira issues by JQL.",
			"inputSchema": map[string]any{"type": "object", "properties": map[string]any{"jql": map[string]any{"type": "string"}}},
			"method":      "djtouchette.jira.search",
		}},
	}}
	answeringProvider(t, ctx, busURL, map[string]any{
		"plugins.tools":           catalogJSON,
		"djtouchette.jira.search": nil, // echo, so the call-routing assert below works
	})

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	catalog := newPluginCatalog(client)
	// Deterministic: one refresh instead of the poll loop. The provider may
	// still be registering, so retry briefly.
	deadline := time.Now().Add(5 * time.Second)
	for {
		catalog.refresh(ctx)
		if byID, _ := catalog.snapshot(); len(byID) > 0 || time.Now().After(deadline) {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	if byID, _ := catalog.snapshot(); len(byID) == 0 {
		t.Fatal("catalog never loaded from plugins.tools")
	}

	cache := newServerCache(client, catalog, tierServers(client))

	// A view token WITH the plugin grant sees the tool (and help lists it).
	granted := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeView, Plugins: []string{"djtouchette.jira"}})
	cs := connectTo(t, ctx, granted)
	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("ListTools: %v", err)
	}
	if !hasTool(tools.Tools, "djtouchette_jira_search") {
		t.Fatalf("granted server missing plugin tool: %v", toolNames(tools.Tools))
	}
	if hasTool(tools.Tools, "spawn_agent") {
		t.Errorf("plugin grant must not widen the tier: view server has spawn_agent")
	}

	// The call forwards to the plugin's bus method with the raw arguments.
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "djtouchette_jira_search",
		Arguments: map[string]any{"jql": "project = X"},
	})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	if res.IsError {
		t.Fatalf("plugin tool errored: %s", textOf(res))
	}
	text := textOf(res)
	if !strings.Contains(text, `"method":"djtouchette.jira.search"`) || !strings.Contains(text, `"jql":"project = X"`) {
		t.Errorf("call did not forward to the plugin method with params: %s", text)
	}

	// help on the granted server lists the plugins group.
	help, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "help", Arguments: map[string]any{"topic": "plugins"}})
	if err != nil {
		t.Fatalf("help plugins: %v", err)
	}
	if !strings.Contains(textOf(help), "djtouchette_jira_search") {
		t.Errorf("help does not list the granted plugin tool: %s", textOf(help))
	}

	// No plugin grant → no plugin tool, even at operator. Opt-in, not ambient.
	ungrantedOp := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator})
	cs2 := connectTo(t, ctx, ungrantedOp)
	tools2, err := cs2.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("ListTools operator: %v", err)
	}
	if hasTool(tools2.Tools, "djtouchette_jira_search") {
		t.Errorf("ungranted operator token must not see plugin tools")
	}

	// A "*" grant resolves to every catalog plugin.
	starServer := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeTriage, Plugins: []string{"*"}})
	cs3 := connectTo(t, ctx, starServer)
	tools3, _ := cs3.ListTools(ctx, nil)
	if !hasTool(tools3.Tools, "djtouchette_jira_search") {
		t.Errorf("star grant did not resolve to catalog plugins: %v", toolNames(tools3.Tools))
	}
}

func TestSanitizeMCPName(t *testing.T) {
	cases := map[string]string{
		"djtouchette.jira": "djtouchette_jira",
		"A.B-C":            "a_b_c",
		"..x..":            "x",
	}
	for in, want := range cases {
		if got := sanitizeMCPName(in); got != want {
			t.Errorf("sanitizeMCPName(%q) = %q, want %q", in, got, want)
		}
	}
}
