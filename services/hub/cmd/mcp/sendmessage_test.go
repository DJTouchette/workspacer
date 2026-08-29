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

// send_message's fromSessionId is the whole of a worker→manager message's
// identity: the provider composes the "[fleet] session:<id> (<label>) says:"
// header from it, and a field dropped anywhere on the way there does not fail —
// it silently delivers an ANONYMOUS message, which is exactly the drift this
// pins (the desktop provider used to drop it while the brain honoured it).
//
// A triage server on purpose: agents.sendMessage is a triage-tier method, and
// triage is the tier a dispatched worker actually holds. Chain: MCP client →
// send_message handler → busclient → a REAL hub bus → an echoing provider.
func TestSendMessageForwardsFromSessionID(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{"agents.sendMessage": nil}) // nil = echo method+params

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	cs := connectTo(t, ctx, newServer(client, authtoken.ScopeTriage))

	res := callTool(t, ctx, cs, "send_message", map[string]any{
		"sessionId":     "MANAGER",
		"text":          "phase 1 landed",
		"fromSessionId": "WORKER",
	})
	if res.IsError {
		t.Fatalf("send_message errored: %s", textOf(res))
	}

	var echo struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal([]byte(textOf(res)), &echo); err != nil {
		t.Fatalf("expected the provider echo, got %q (%v)", textOf(res), err)
	}
	if echo.Method != "agents.sendMessage" {
		t.Fatalf("send_message routed to %q, want agents.sendMessage", echo.Method)
	}
	var params map[string]any
	if err := json.Unmarshal(echo.Params, &params); err != nil {
		t.Fatal(err)
	}
	for k, want := range map[string]string{
		"sessionId":     "MANAGER",
		"text":          "phase 1 landed",
		"fromSessionId": "WORKER",
	} {
		if params[k] != want {
			t.Errorf("send_message forwarded %s=%v, want %q", k, params[k], want)
		}
	}
}
