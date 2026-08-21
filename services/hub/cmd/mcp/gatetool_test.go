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

// set_approval_gate's chronic confusion: the gate is a workspacer-side hold,
// the Claude permission mode is the session's own prompting policy, and from
// outside they look identical — so "gate off, ok:true, still prompting" reads
// as a bug. The behavior stays unchanged; the RESPONSE now reports the
// session's current permission mode plus a note stating the distinction.

func gateSession(t *testing.T, ctx context.Context, answers map[string]any) *mcp.ClientSession {
	t.Helper()
	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, answers)

	client := busclient.New(busURL, "")
	go client.Run(ctx)
	return connectTo(t, ctx, newServer(client, authtoken.ScopeOperator))
}

func callGate(t *testing.T, ctx context.Context, cs *mcp.ClientSession, args map[string]any) map[string]any {
	t.Helper()
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "set_approval_gate", Arguments: args})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	var text string
	for _, c := range res.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			text += tc.Text
		}
	}
	if res.IsError {
		t.Fatalf("set_approval_gate errored: %s", text)
	}
	var out map[string]any
	if err := json.Unmarshal([]byte(text), &out); err != nil {
		t.Fatalf("gate response is not JSON (%v): %s", err, text)
	}
	return out
}

// TestSetApprovalGateReportsThePermissionMode: gate off keeps the provider's
// own fields (ok/gate_enabled — the existing convention) and adds the session's
// live permission mode plus a note that the session still prompts per it.
func TestSetApprovalGateReportsThePermissionMode(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := gateSession(t, ctx, map[string]any{
		"claude.gate":       map[string]any{"ok": true, "session_id": "s1", "gate_enabled": false},
		"sessions.snapshot": map[string]any{"sessionId": "s1", "livePermissionMode": "default"},
	})
	out := callGate(t, ctx, cs, map[string]any{"sessionId": "s1", "on": false})

	if out["ok"] != true || out["gate_enabled"] != false {
		t.Fatalf("provider fields must survive the enrichment, got %v", out)
	}
	if out["permissionMode"] != "default" {
		t.Fatalf("response must carry the session's current permission mode, got %v", out)
	}
	note, _ := out["note"].(string)
	if !strings.Contains(note, "still prompts") || !strings.Contains(note, "permission mode") {
		t.Fatalf("gate-off note must state the gate/permission-mode distinction, got %q", note)
	}
}

// TestSetApprovalGateSurvivesAMissingSnapshot: the mode lookup is best-effort —
// an unanswerable sessions.snapshot degrades to "unknown", never to a failed
// gate call (the gate itself already took effect).
func TestSetApprovalGateSurvivesAMissingSnapshot(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := gateSession(t, ctx, map[string]any{
		"claude.gate": map[string]any{"ok": true, "session_id": "s1", "gate_enabled": true},
		// no sessions.snapshot provider registered
	})
	out := callGate(t, ctx, cs, map[string]any{"sessionId": "s1", "on": true})

	if out["ok"] != true {
		t.Fatalf("gate result must survive a failed mode lookup, got %v", out)
	}
	if out["permissionMode"] != "unknown" {
		t.Fatalf("unresolvable mode must report \"unknown\", got %v", out)
	}
}

// TestSetApprovalGateReadsTheSpawnSettingsMode: a session with no live hook
// telemetry still resolves its mode from the snapshot's spawn-time settings —
// the desktop store's other spelling.
func TestSetApprovalGateReadsTheSpawnSettingsMode(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := gateSession(t, ctx, map[string]any{
		"claude.gate": map[string]any{"ok": true, "session_id": "s1", "gate_enabled": true},
		"sessions.snapshot": map[string]any{
			"sessionId": "s1",
			"settings":  map[string]any{"permissionMode": "plan"},
		},
	})
	out := callGate(t, ctx, cs, map[string]any{"sessionId": "s1", "on": true})
	if out["permissionMode"] != "plan" {
		t.Fatalf("settings.permissionMode must resolve when no live mode exists, got %v", out)
	}
}
