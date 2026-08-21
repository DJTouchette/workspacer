package main

import (
	"bytes"
	"context"
	"encoding/json"
	"log"
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

// Full-access dispatch, facade half. The facade multiplexes every session
// token over ONE trusted bus connection, so the hub stamps `yoloGranted` for
// that host-token conn regardless of which session is riding it — the
// per-record YoloAllowed grant can only be honored HERE, where resolveRecord
// resolved the token. Twin of profilegrant_test.go, but the ungranted path
// DEGRADES silently (skipPermissions clamped off) rather than refusing, matching
// the "remote spawns never auto-bypass" doctrine. These tests run the real
// chain: MCP client → spawn_agent handler → busclient → a REAL hub bus → an
// echoing provider, and read what the provider actually received.

// yoloGrantSession builds a facade server for one record's full-access grant
// against a live hub with an echoing agents.spawn provider.
func yoloGrantSession(t *testing.T, ctx context.Context, yolo bool) *mcp.ClientSession {
	t.Helper()
	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{"agents.spawn": nil}) // nil = echo method+params

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	server := newServerWithGrants(client, authtoken.ScopeOperator, nil, nil, yolo)
	return connectTo(t, ctx, server)
}

// spawnEchoParams runs a spawn_agent call and decodes the params the provider
// received (the facade's forwarded frame, after any facade-side clamp).
func spawnEchoParams(t *testing.T, ctx context.Context, cs *mcp.ClientSession, args map[string]any) map[string]any {
	t.Helper()
	text, isErr := callSpawn(t, ctx, cs, args)
	if isErr {
		t.Fatalf("spawn_agent errored unexpectedly: %s", text)
	}
	var echo struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal([]byte(text), &echo); err != nil {
		t.Fatalf("expected the provider echo, got %q (%v)", text, err)
	}
	var params map[string]any
	if err := json.Unmarshal(echo.Params, &params); err != nil {
		t.Fatal(err)
	}
	return params
}

// TestSpawnAgentClampsSkipPermissionsForAnUngrantedSession: a session token
// WITHOUT the full-access grant has skipPermissions clamped to false BEFORE the
// forward, so the hub's yoloGranted stamp (which it applies to the facade's
// host-token conn regardless) never meets a live bypass request. The clamp is
// silent — the spawn still succeeds, just with approvals on.
func TestSpawnAgentClampsSkipPermissionsForAnUngrantedSession(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := yoloGrantSession(t, ctx, false)
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "skipPermissions": true})
	// omitempty on the wire: a clamped-false bool is dropped, so the provider
	// must see NO truthy skipPermissions.
	if params["skipPermissions"] == true {
		t.Fatalf("ungranted session's skipPermissions was not clamped before forward: %v", params)
	}
}

// TestSpawnAgentForwardsSkipPermissionsForAGrantedSession: a session token WITH
// the full-access grant forwards skipPermissions untouched → the hub stamps
// yoloGranted → the provider (here, the echo) receives the live request. The
// positive half, end to end.
func TestSpawnAgentForwardsSkipPermissionsForAGrantedSession(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := yoloGrantSession(t, ctx, true)
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "skipPermissions": true})
	if params["skipPermissions"] != true {
		t.Fatalf("granted session's skipPermissions must ride through the facade: %v", params)
	}
	if params["yoloGranted"] != true {
		t.Fatalf("hub did not stamp yoloGranted on the facade's forwarded spawn: %v", params)
	}
}

// TestSpawnAgentClampLogsTheStrip: the clamp degrades the CALLER silently (the
// spawn still succeeds, approvals on), but the strip itself must be loggable —
// a dropped bypass was previously undiagnosable. One line, naming the calling
// token (label from the request context; "untokened" over the in-memory test
// transport) and the requested agent label; and NO line when nothing was
// requested, so the log only speaks when a bypass was actually dropped.
func TestSpawnAgentClampLogsTheStrip(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	var buf bytes.Buffer
	prev := log.Writer()
	log.SetOutput(&buf)
	defer log.SetOutput(prev)

	cs := yoloGrantSession(t, ctx, false)
	_ = spawnEchoParams(t, ctx, cs, map[string]any{
		"cwd": "/tmp", "skipPermissions": true, "label": "worker-1",
	})
	out := buf.String()
	if !strings.Contains(out, "skipPermissions requested without the full-access grant") ||
		!strings.Contains(out, `"worker-1"`) || !strings.Contains(out, "untokened") {
		t.Fatalf("stripped bypass must be logged with token + agent label, got:\n%s", out)
	}

	buf.Reset()
	_ = spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "label": "worker-2"})
	if strings.Contains(buf.String(), "full-access grant") {
		t.Fatalf("a spawn that requested no bypass must not log a clamp line, got:\n%s", buf.String())
	}
}

// TestServerCacheSeparatesYoloGrants: two records at the same tier with
// different full-access grants must never share a server — the clamp is closed
// over the build, so a shared server IS a shared grant. Also pins that a
// yolo-only record does NOT collapse onto the shared (clamped) tier server.
func TestServerCacheSeparatesYoloGrants(t *testing.T) {
	client := busclient.New("ws://127.0.0.1:0/bus", "")
	cache := newServerCache(client, newPluginCatalog(client), tierServers(client))

	plain := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator})
	if plain != cache.base[authtoken.ScopeOperator] {
		t.Fatal("a grantless record should get the shared tier server")
	}
	full := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, YoloAllowed: true})
	if full == plain {
		t.Fatal("a full-access record must not collapse onto the grantless (clamped) tier server")
	}
	again := cache.serverFor(authtoken.Record{Scope: authtoken.ScopeOperator, YoloAllowed: true})
	if again != full {
		t.Fatal("same grant should hit the cache, not rebuild")
	}
}
