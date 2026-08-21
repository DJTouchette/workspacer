package main

import (
	"bytes"
	"context"
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

// Omitted-skipPermissions default resolution, facade half. The desktop spawn
// dialog pre-selects the config default (claude.skipPermissionsDefault / a
// bypass defaultPermissionMode); a spawn_agent call that omits the field must
// resolve the SAME default — through the SAME grant gate as an explicit
// request, so the operator's default never escalates an ungranted session
// token. These run the real chain like yologrant_test.go: MCP client →
// spawn_agent handler → busclient → a REAL hub bus → an echoing provider that
// also answers config.get.

// spawnDefaultsSession is yoloGrantSession with a config.get answer beside the
// agents.spawn echo, so the facade's default resolution has a config to read.
func spawnDefaultsSession(t *testing.T, ctx context.Context, yolo bool, claudeCfg map[string]any) *mcp.ClientSession {
	t.Helper()
	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{
		"agents.spawn": nil, // nil = echo method+params
		"config.get":   map[string]any{"claude": claudeCfg},
	})

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	server := newServerWithGrants(client, authtoken.ScopeOperator, nil, nil, yolo)
	return connectTo(t, ctx, server)
}

// TestSpawnAgentOmittedSkipResolvesConfigDefaultForAGrantedSession: granted
// token + omitted field + claude.skipPermissionsDefault:true → the worker
// spawns bypassed, exactly like the desktop dialog's pre-selected toggle.
func TestSpawnAgentOmittedSkipResolvesConfigDefaultForAGrantedSession(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{"skipPermissionsDefault": true})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if params["skipPermissions"] != true {
		t.Fatalf("granted session + config default on: omitted skipPermissions must resolve to true, got %v", params)
	}
	if params["yoloGranted"] != true {
		t.Fatalf("hub did not stamp yoloGranted on the defaulted spawn: %v", params)
	}
}

// TestSpawnAgentOmittedSkipHonorsABypassDefaultPermissionMode: the other config
// spelling of the same default — defaultPermissionMode:"bypassPermissions" with
// the toggle off.
func TestSpawnAgentOmittedSkipHonorsABypassDefaultPermissionMode(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{
		"skipPermissionsDefault": false,
		"defaultPermissionMode":  "bypassPermissions",
	})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if params["skipPermissions"] != true {
		t.Fatalf("a bypass defaultPermissionMode must resolve the omitted field to true, got %v", params)
	}
}

// TestSpawnAgentConfigDefaultIsClampedAndLoggedForAnUngrantedSession: the
// SECURITY half. The config default passes the SAME gate as an explicit
// request — without the full-access grant it is clamped, the strip is logged
// with its config-default provenance, and the wire carries an EXPLICIT false
// (never nil): the hub stamps yoloGranted on the facade's trusted host-token
// connection regardless of session, so a nil left on the wire would let the
// provider's own default resolution escalate the ungranted token.
func TestSpawnAgentConfigDefaultIsClampedAndLoggedForAnUngrantedSession(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	var buf bytes.Buffer
	prev := log.Writer()
	log.SetOutput(&buf)
	defer log.SetOutput(prev)

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{"skipPermissionsDefault": true})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "label": "worker-d"})

	v, present := params["skipPermissions"]
	if !present || v != false {
		t.Fatalf("ungranted session must forward an EXPLICIT skipPermissions:false (got present=%v value=%v) — an omitted field re-resolves the config default provider-side under the facade's yoloGranted stamp", present, v)
	}
	out := buf.String()
	if !strings.Contains(out, "config-defaulted") || !strings.Contains(out, "full-access grant") ||
		!strings.Contains(out, `"worker-d"`) {
		t.Fatalf("clamped config default must be logged with its provenance + agent label, got:\n%s", out)
	}
}

// TestSpawnAgentOmittedSkipWithDefaultOffStaysOff: default off + omitted field
// → approvals on, and still an explicit false on the wire.
func TestSpawnAgentOmittedSkipWithDefaultOffStaysOff(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{"skipPermissionsDefault": false})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if v, present := params["skipPermissions"]; !present || v != false {
		t.Fatalf("default off: omitted skipPermissions must forward explicit false, got present=%v value=%v", present, v)
	}
}

// TestSpawnAgentExplicitFalseBeatsTheConfigDefault: an explicit caller value —
// including false — always wins over the config default.
func TestSpawnAgentExplicitFalseBeatsTheConfigDefault(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{"skipPermissionsDefault": true})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "skipPermissions": false})
	if v, present := params["skipPermissions"]; !present || v != false {
		t.Fatalf("explicit false must beat the config default, got present=%v value=%v", present, v)
	}
}
