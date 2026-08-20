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

// The open_terminal facade tool (visible-terminal path) must reach the
// terminals.open capability with the caller's fields intact — the seam a unit
// test of openManagedTerminal (renderer) cannot see. Chain: MCP client →
// open_terminal handler → busclient → a REAL hub bus → an echoing provider that
// returns the method + params it received.
func TestOpenTerminalForwardsToTerminalsOpen(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{"terminals.open": nil}) // nil = echo method+params

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	// Operator tier: open_terminal is operator-only (the wildcard scope), the
	// tier the Fleet Manager and its server-runner workers hold.
	cs := connectTo(t, ctx, newServer(client, authtoken.ScopeOperator))

	res := callTool(t, ctx, cs, "open_terminal", map[string]any{
		"cwd":             "/home/u/Work/preheat",
		"command":         "npm run dev",
		"label":           "preheat dev server",
		"parentSessionId": "MGR",
	})
	if res.IsError {
		t.Fatalf("open_terminal errored: %s", textOf(res))
	}

	var echo struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal([]byte(textOf(res)), &echo); err != nil {
		t.Fatalf("expected the provider echo, got %q (%v)", textOf(res), err)
	}
	if echo.Method != "terminals.open" {
		t.Fatalf("open_terminal routed to %q, want terminals.open", echo.Method)
	}
	var params map[string]any
	if err := json.Unmarshal(echo.Params, &params); err != nil {
		t.Fatal(err)
	}
	for k, want := range map[string]string{
		"cwd":             "/home/u/Work/preheat",
		"command":         "npm run dev",
		"label":           "preheat dev server",
		"parentSessionId": "MGR",
	} {
		if params[k] != want {
			t.Errorf("open_terminal forwarded %s=%v, want %q", k, params[k], want)
		}
	}
}

// A view-tier session cannot open a terminal at all — the tool is operator-only,
// so it is not even registered on a view server (fail-closed, not a runtime
// refusal). This pins that open_terminal never leaks below operator.
func TestOpenTerminalIsOperatorOnly(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	client := busclient.New(busURL, "")
	go client.Run(ctx)

	cs := connectTo(t, ctx, newServer(client, authtoken.ScopeView))
	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, tool := range tools.Tools {
		if tool.Name == "open_terminal" {
			t.Fatal("open_terminal is exposed at the view tier — it must be operator-only")
		}
	}
}
