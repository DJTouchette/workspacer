package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Legacy profile metadata, facade half. Profiles are ambient for authenticated
// spawns; persisted profilesAllowed/profileGranted shapes must neither narrow
// the call nor reappear as authoritative stamps.

// spawnGrantSession builds a facade server for one record's grants against a
// live hub with an echoing agents.spawn provider, and returns the MCP session.
func spawnGrantSession(t *testing.T, ctx context.Context, profiles []string) *mcp.ClientSession {
	t.Helper()
	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{"agents.spawn": nil}) // nil = echo method+params

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	server := newServerWithGrants(client, authtoken.ScopeOperator, nil, profiles, false)
	return connectTo(t, ctx, server)
}

func callSpawn(t *testing.T, ctx context.Context, cs *mcp.ClientSession, args map[string]any) (string, bool) {
	t.Helper()
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "spawn_agent", Arguments: args})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	var text string
	for _, c := range res.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			text += tc.Text
		}
	}
	return text, res.IsError
}

// The selected profile flows end to end with no separate grant stamp.
func TestSpawnAgentForwardsProfileWithoutAGrantStamp(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnGrantSession(t, ctx, []string{"work", "personal"})
	text, isErr := callSpawn(t, ctx, cs, map[string]any{"cwd": "/tmp", "profileId": "work"})
	if isErr {
		t.Fatalf("selected profileId was refused: %s", text)
	}
	var echo struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal([]byte(text), &echo); err != nil {
		t.Fatalf("expected the provider echo, got %q (%v)", text, err)
	}
	if echo.Method != "agents.spawn" {
		t.Fatalf("forwarded method %q", echo.Method)
	}
	var params map[string]any
	if err := json.Unmarshal(echo.Params, &params); err != nil {
		t.Fatal(err)
	}
	if params["profileId"] != "work" {
		t.Fatalf("provider did not receive the selected profileId: %v", params)
	}
	if _, stamped := params["profileGranted"]; stamped {
		t.Fatalf("obsolete profile grant stamp reached the provider: %v", params)
	}
}

// The retired profileGranted compatibility stamp is not a public tool input.
func TestSpawnAgentCallerCannotSupplyProfileGranted(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnGrantSession(t, ctx, nil)
	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, tl := range tools.Tools {
		if tl.Name != "spawn_agent" {
			continue
		}
		schema, _ := json.Marshal(tl.InputSchema)
		if strings.Contains(string(schema), "profileGranted") {
			t.Fatalf("spawn_agent's input schema must not offer retired profileGranted: %s", schema)
		}
		return
	}
	t.Fatal("spawn_agent tool not found on an operator server")
}

// Legacy profile metadata does not change the server cache key.
func TestServerCacheIgnoresLegacyProfileGrants(t *testing.T) {
	client := busclient.New("ws://127.0.0.1:0/bus", "")
	cache := newServerCache(client, newPluginCatalog(client), tierServers(client))

	plain := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator})
	if plain != cache.base[authtoken.ScopeOperator] {
		t.Fatal("a grantless record should get the shared tier server")
	}
	mgr := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, ProfilesAllowed: []string{"work"}})
	if mgr != plain {
		t.Fatal("a legacy profile grant changed an otherwise identical server")
	}
	other := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, ProfilesAllowed: []string{"personal"}})
	if other != mgr {
		t.Fatal("legacy profile grant contents changed the server cache key")
	}
	again := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, ProfilesAllowed: []string{"work"}})
	if again != mgr {
		t.Fatal("same grant should hit the cache, not rebuild")
	}
}
