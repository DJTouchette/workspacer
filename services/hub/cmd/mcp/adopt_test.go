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

// list_orphans is the discovery half of succession — the read a successor makes
// when its predecessor CRASHED and left no handoff file to name itself in. It
// must route to agents.orphans and it must carry NO caller params: the moment it
// takes a session id it stops being "what is orphaned here" and starts being a
// question whose answer the caller can steer.
func TestListOrphansRoutesToAgentsOrphans(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{"agents.orphans": nil}) // nil = echo method+params

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	cs := connectTo(t, ctx, newServer(client, authtoken.ScopeOperator))

	res := callTool(t, ctx, cs, "list_orphans", map[string]any{})
	if res.IsError {
		t.Fatalf("list_orphans errored: %s", textOf(res))
	}
	var echo struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal([]byte(textOf(res)), &echo); err != nil {
		t.Fatalf("expected the provider echo, got %q (%v)", textOf(res), err)
	}
	if echo.Method != "agents.orphans" {
		t.Fatalf("list_orphans routed to %q, want agents.orphans", echo.Method)
	}
	var params map[string]any
	if err := json.Unmarshal(echo.Params, &params); err != nil {
		t.Fatal(err)
	}
	if len(params) != 0 {
		t.Errorf("list_orphans forwarded params %v, want none", params)
	}
}

// Operator-only for the same reason adopt_workers is: it exists only to feed
// adopt_workers, and a view scout or a phone token that could enumerate every
// dead manager's label and directory would be reading the fleet's history
// through a door the tier was defined not to have.
func TestListOrphansIsOperatorOnly(t *testing.T) {
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
			if tool.Name == "list_orphans" {
				t.Fatalf("list_orphans is exposed at the %s tier — it must be operator-only", scope)
			}
		}
	}
}
