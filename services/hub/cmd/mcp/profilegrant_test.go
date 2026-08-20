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

// Profile-aware dispatch, facade half (FLEET_MANAGER_SPIKE §6a). The facade
// multiplexes every session token over ONE trusted bus connection, so the hub
// can only see the facade's credential — the per-record profilesAllowed check
// has to happen here, where resolveRecord resolved the token. These tests run
// the real chain: in-memory MCP client → spawn_agent handler → busclient → a
// REAL hub bus (whose router stamps profileGranted) → an echoing provider.

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

	server := newServerWithGrants(client, authtoken.ScopeOperator, nil, profiles)
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

// TestSpawnAgentRefusesAnUngrantedProfile: the refusal happens IN the facade —
// nothing is forwarded, so the error names the grant rather than echoing a
// degraded spawn (silently landing on the default account is the failure mode
// this exists to prevent).
func TestSpawnAgentRefusesAnUngrantedProfile(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	// A manager blessed for "work" only.
	cs := spawnGrantSession(t, ctx, []string{"work"})
	text, isErr := callSpawn(t, ctx, cs, map[string]any{"cwd": "/tmp", "profileId": "personal"})
	if !isErr {
		t.Fatalf("ungranted profileId was not refused; result: %s", text)
	}
	if !strings.Contains(text, `"personal"`) || !strings.Contains(text, "not granted") {
		t.Fatalf("refusal should name the profile and the grant, got: %s", text)
	}
	if strings.Contains(text, "agents.spawn") && strings.Contains(text, "params") {
		t.Fatalf("refusal appears to have forwarded to the hub anyway: %s", text)
	}

	// A record with NO grant at all (the untokened/static operator default)
	// refuses every profileId — this is fail-closed, and it is also strictly
	// better than the old silent degradation.
	cs = spawnGrantSession(t, ctx, nil)
	text, isErr = callSpawn(t, ctx, cs, map[string]any{"cwd": "/tmp", "profileId": "work"})
	if !isErr || !strings.Contains(text, "not granted") {
		t.Fatalf("grantless record should refuse any profileId, got (isErr=%v): %s", isErr, text)
	}
}

// TestSpawnAgentForwardsAGrantedProfileAndTheHubStamps: the positive half,
// end to end — a granted id passes the facade, rides the facade's trusted bus
// connection, and arrives at the provider WITH the hub's profileGranted stamp.
func TestSpawnAgentForwardsAGrantedProfileAndTheHubStamps(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnGrantSession(t, ctx, []string{"work", "personal"})
	text, isErr := callSpawn(t, ctx, cs, map[string]any{"cwd": "/tmp", "profileId": "work"})
	if isErr {
		t.Fatalf("granted profileId was refused: %s", text)
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
		t.Fatalf("provider did not receive the granted profileId: %v", params)
	}
	if params["profileGranted"] != true {
		t.Fatalf("hub did not stamp profileGranted on the facade's forwarded spawn: %v", params)
	}
}

// TestSpawnAgentCallerCannotSupplyProfileGranted: profileGranted is not a tool
// input at all — the schema has no such property, so a session cannot even
// SPEAK the stamp toward the facade; and if a permissive client got it through,
// the hub deletes caller-supplied copies before any provider sees them (pinned
// on the hub side by TestProfileGrantSpoofedFieldsNeverReachTheProvider).
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
			t.Fatalf("spawn_agent's input schema must not offer profileGranted (hub-stamped only): %s", schema)
		}
		return
	}
	t.Fatal("spawn_agent tool not found on an operator server")
}

// TestServerCacheSeparatesProfileGrants: two records at the same tier with
// different account grants must never share a server — the grant check is
// closed over the build, so a shared server IS a shared grant.
func TestServerCacheSeparatesProfileGrants(t *testing.T) {
	client := busclient.New("ws://127.0.0.1:0/bus", "")
	cache := newServerCache(client, newPluginCatalog(client), tierServers(client))

	plain := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator})
	if plain != cache.base[authtoken.ScopeOperator] {
		t.Fatal("a grantless record should get the shared tier server")
	}
	mgr := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, ProfilesAllowed: []string{"work"}})
	if mgr == plain {
		t.Fatal("a profile-granted record must not collapse onto the grantless tier server")
	}
	other := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, ProfilesAllowed: []string{"personal"}})
	if other == mgr {
		t.Fatal("records with different profile grants shared a server (cache key ignores the grant)")
	}
	again := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, ProfilesAllowed: []string{"work"}})
	if again != mgr {
		t.Fatal("same grant should hit the cache, not rebuild")
	}
}
