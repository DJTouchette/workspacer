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

// TestNotifyCarriesAClickTarget proves the notify tool can say WHERE a
// notification points, not just what it says.
//
// The tool input used to be title + body only, while the capability behind it
// (notifications.post) already accepted a session, a pane, a url and the rest.
// An agent could therefore raise a toast and had to spell out "go and look in
// Settings" in prose, because the click had nowhere to go. This walks the whole
// chain — MCP tool call, facade, hub bus, provider — and checks each field
// arrives under the json name the capability destructures.
func TestNotifyCarriesAClickTarget(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer hub.Close()
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	fakeProvider(t, ctx, busURL, []string{"notifications.post"})

	client := busclient.New(busURL, "")
	go client.Run(ctx)
	server := newServer(client, authtoken.ScopeOperator)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer cs.Close()

	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name: "notify",
		Arguments: map[string]any{
			"title":       "Job proposed: Nightly sync",
			"body":        "It won't run until you approve it.",
			"level":       "info",
			"key":         "job-proposal-p1",
			"sessionId":   "s1",
			"paneType":    "settings",
			"paneSection": "jobs",
			"url":         "https://example.test/docs",
			"silent":      true,
			"inAppOnly":   true,
		},
	})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	if res.IsError {
		t.Fatalf("tool returned error: %v", textOf(res))
	}

	// The provider echoes {method, params}; unwrap the params it actually got.
	var echoed struct {
		Method string         `json:"method"`
		Params map[string]any `json:"params"`
	}
	if err := json.Unmarshal([]byte(textOf(res)), &echoed); err != nil {
		t.Fatalf("unmarshal echo %q: %v", textOf(res), err)
	}
	if echoed.Method != "notifications.post" {
		t.Fatalf("routed to %q, want notifications.post", echoed.Method)
	}
	// Names, not just presence: a json tag that does not match the capability's
	// own destructure is forwarded and silently ignored, which looks identical
	// to working until someone clicks.
	for field, want := range map[string]any{
		"title":       "Job proposed: Nightly sync",
		"body":        "It won't run until you approve it.",
		"level":       "info",
		"key":         "job-proposal-p1",
		"sessionId":   "s1",
		"paneType":    "settings",
		"paneSection": "jobs",
		"url":         "https://example.test/docs",
		"silent":      true,
		"inAppOnly":   true,
	} {
		if got := echoed.Params[field]; got != want {
			t.Errorf("params[%q] = %v, want %v", field, got, want)
		}
	}
}
