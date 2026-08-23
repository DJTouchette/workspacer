package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"

	"net/http/httptest"
)

// adopt_workers is the successor's half of a manager handover: it must reach
// agents.reparent with BOTH ids the caller named, because the whole call is
// those two ids — a dropped or renamed field would land on the provider's
// "requires { fromSessionId, toSessionId }" refusal, or worse, move nothing and
// report ok. Chain: MCP client → adopt_workers handler → busclient → a REAL hub
// bus → an echoing provider.
func TestAdoptWorkersForwardsBothIds(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{"agents.reparent": nil}) // nil = echo method+params

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	cs := connectTo(t, ctx, newServer(client, authtoken.ScopeOperator))

	res := callTool(t, ctx, cs, "adopt_workers", map[string]any{
		"fromSessionId": "OLD-MGR",
		"toSessionId":   "NEW-MGR",
	})
	if res.IsError {
		t.Fatalf("adopt_workers errored: %s", textOf(res))
	}

	var echo struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal([]byte(textOf(res)), &echo); err != nil {
		t.Fatalf("expected the provider echo, got %q (%v)", textOf(res), err)
	}
	if echo.Method != "agents.reparent" {
		t.Fatalf("adopt_workers routed to %q, want agents.reparent", echo.Method)
	}
	var params map[string]any
	if err := json.Unmarshal(echo.Params, &params); err != nil {
		t.Fatal(err)
	}
	for k, want := range map[string]string{"fromSessionId": "OLD-MGR", "toSessionId": "NEW-MGR"} {
		if params[k] != want {
			t.Errorf("adopt_workers forwarded %s=%v, want %q", k, params[k], want)
		}
	}
}

// Operator-only, fail-closed: agents.reparent is in neither scoped tier's
// allowlist, so the tool is not registered on a view or triage server at all.
// A dispatched worker must never be able to move a manager's fleet onto itself
// — the destination check in the provider (must be a live supervisor) is the
// last line, not the first.
func TestAdoptWorkersIsOperatorOnly(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	client := busclient.New(busURL, "")
	go client.Run(ctx)

	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage} {
		cs := connectTo(t, ctx, newServer(client, scope))
		tools, err := cs.ListTools(ctx, nil)
		if err != nil {
			t.Fatal(err)
		}
		for _, tool := range tools.Tools {
			if tool.Name == "adopt_workers" {
				t.Fatalf("adopt_workers is exposed at the %s tier — it must be operator-only", scope)
			}
		}
	}
}
