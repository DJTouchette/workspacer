package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// spawn_agent's `message` — the new agent's first prompt, carried by the spawn
// instead of a follow-up send_message.
//
// The property under test is that it CANNOT be silently lost. The provider
// acknowledges taking delivery (`messageQueued`); anything else falls back to
// the old two-call form, and a failure of THAT is reported rather than dressed
// up as a successful dispatch. An agent running with no instructions looks
// exactly like a wedged one, which is the whole reason this is not fire-and-forget.

// spawnHub is a scripted bus for the spawn_agent path: it answers agents.spawn
// (optionally acknowledging the prompt) and agents.sendMessage, and records
// every call so a test can assert on what was actually forwarded.
type spawnHub struct {
	calls []busCall
	// confirms mirrors a current provider: the spawn result carries
	// `messageQueued` when a message rode the params. False mirrors a provider
	// that does not know the field at all — an older federated peer or a
	// lagging headless brain, whose spawn looks perfectly successful while the
	// prompt goes nowhere.
	confirms bool
	// noSessionID mirrors a provider whose result cannot be addressed, so
	// neither the spawn nor a fallback can deliver anything.
	noSessionID bool
	sendErr     bool
}

func (h *spawnHub) Call(_ context.Context, method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var decoded map[string]any
	_ = json.Unmarshal(raw, &decoded)
	h.calls = append(h.calls, busCall{method: method, params: decoded})
	switch method {
	case "agents.spawn":
		if h.noSessionID {
			return json.RawMessage(`{}`), nil
		}
		msg, _ := decoded["message"].(string)
		if h.confirms && strings.TrimSpace(msg) != "" {
			return json.RawMessage(`{"sessionId":"new-1","messageQueued":true}`), nil
		}
		return json.RawMessage(`{"sessionId":"new-1"}`), nil
	case "agents.sendMessage":
		if h.sendErr {
			return nil, errFake
		}
		return json.RawMessage(`{"ok":true}`), nil
	case "config.get":
		return json.RawMessage(`{"claude":{}}`), nil
	}
	return json.RawMessage(`{}`), nil
}

func (h *spawnHub) call(method string) *busCall {
	for i := range h.calls {
		if h.calls[i].method == method {
			return &h.calls[i]
		}
	}
	return nil
}

func callSpawnAgent(t *testing.T, hub *spawnHub, args map[string]any) *mcp.CallToolResult {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	srv := mcp.NewServer(&mcp.Implementation{Name: "workspacer-test", Version: "v1"}, nil)
	b := &build{
		s:     srv,
		scope: authtoken.ScopeOperator,
		allow: authtoken.ScopeOperator.Methods(),
		caller: func(ctx context.Context, method string, params any) (json.RawMessage, error) {
			return hub.Call(ctx, method, params)
		},
	}
	addSpawnTool(b, "spawn_agent", "spawn", "agents.spawn")

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := srv.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer cs.Close()

	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "spawn_agent", Arguments: args})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	return res
}

func TestSpawnAgentCarriesTheFirstMessageOnTheSpawn(t *testing.T) {
	hub := &spawnHub{confirms: true}
	res := callSpawnAgent(t, hub, map[string]any{"cwd": "/tmp", "message": "ship the thing"})
	if res.IsError {
		t.Fatalf("unexpected error: %s", resultText(res))
	}
	spawn := hub.call("agents.spawn")
	if spawn == nil {
		t.Fatal("no agents.spawn call")
	}
	if got, _ := spawn.params["message"].(string); got != "ship the thing" {
		t.Errorf("spawn message = %q, want the dispatch text", got)
	}
	// One call, not two: that is the round trip this removes.
	if send := hub.call("agents.sendMessage"); send != nil {
		t.Errorf("a confirmed spawn must not also send separately, got %v", send.params)
	}
}

func TestSpawnAgentFallsBackWhenTheProviderDoesNotConfirm(t *testing.T) {
	hub := &spawnHub{confirms: false}
	res := callSpawnAgent(t, hub, map[string]any{"cwd": "/tmp", "message": "ship the thing"})
	if res.IsError {
		t.Fatalf("unexpected error: %s", resultText(res))
	}
	send := hub.call("agents.sendMessage")
	if send == nil {
		t.Fatal("an unconfirmed spawn must fall back to sending the prompt itself")
	}
	if id, _ := send.params["sessionId"].(string); id != "new-1" {
		t.Errorf("prompt sent to %q, want the new session", id)
	}
	if text, _ := send.params["text"].(string); text != "ship the thing" {
		t.Errorf("fallback text = %q, want the dispatch verbatim", text)
	}
}

func TestSpawnAgentReportsAnUndeliverableFirstMessage(t *testing.T) {
	hub := &spawnHub{sendErr: true}
	res := callSpawnAgent(t, hub, map[string]any{"cwd": "/tmp", "message": "ship the thing"})
	if !res.IsError {
		t.Fatalf("want an error result, got: %s", resultText(res))
	}
	if !strings.Contains(resultText(res), "idle") {
		t.Errorf("the failure must say the agent is idle and needs the task, got: %s", resultText(res))
	}
}

// A result with no addressable session id cannot be recovered from at all, so
// it is reported rather than assumed to have worked.
func TestSpawnAgentReportsAnUnaddressableSpawnThatCarriedAMessage(t *testing.T) {
	hub := &spawnHub{noSessionID: true}
	res := callSpawnAgent(t, hub, map[string]any{"cwd": "/tmp", "message": "ship the thing"})
	if !res.IsError || !strings.Contains(resultText(res), "sessionId") {
		t.Errorf("want a refusal naming the missing sessionId, got: %s", resultText(res))
	}
}

// A spawn with no message must be untouched by any of this — same one call, same
// result, no send.
func TestSpawnAgentWithoutAMessageIsUnchanged(t *testing.T) {
	hub := &spawnHub{}
	res := callSpawnAgent(t, hub, map[string]any{"cwd": "/tmp"})
	if res.IsError {
		t.Fatalf("unexpected error: %s", resultText(res))
	}
	if send := hub.call("agents.sendMessage"); send != nil {
		t.Errorf("a spawn with no message must send nothing, got %v", send.params)
	}
	if _, ok := hub.call("agents.spawn").params["message"]; ok {
		t.Error("an unset message must not ride the wire at all")
	}
}
