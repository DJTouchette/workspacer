package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// busFrame mirrors the hub wire shape for the test provider.
type busFrame struct {
	Op      string          `json:"op"`
	ID      string          `json:"id,omitempty"`
	Method  string          `json:"method,omitempty"`
	Methods []string        `json:"methods,omitempty"`
	Params  json.RawMessage `json:"params,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   string          `json:"error,omitempty"`
}

// fakeProvider connects to the real hub bus, registers methods, and answers
// calls by echoing the method + params back as the result. It is the stand-in
// for the Electron main process.
func fakeProvider(t *testing.T, ctx context.Context, busURL string, methods []string) {
	t.Helper()
	conn, _, err := websocket.Dial(ctx, busURL, nil)
	if err != nil {
		t.Fatalf("provider dial: %v", err)
	}
	reg, _ := json.Marshal(busFrame{Op: "register", Methods: methods})
	if err := conn.Write(ctx, websocket.MessageText, reg); err != nil {
		t.Fatalf("provider register: %v", err)
	}
	go func() {
		defer conn.CloseNow()
		for {
			_, data, err := conn.Read(ctx)
			if err != nil {
				return
			}
			var f busFrame
			if json.Unmarshal(data, &f) != nil || f.Op != "call" {
				continue
			}
			out, _ := json.Marshal(map[string]any{
				"method": f.Method,
				"params": json.RawMessage(nonNil(f.Params)),
			})
			reply, _ := json.Marshal(busFrame{Op: "result", ID: f.ID, Result: out})
			_ = conn.Write(ctx, websocket.MessageText, reply)
		}
	}()
}

func nonNil(r json.RawMessage) json.RawMessage {
	if len(r) == 0 {
		return json.RawMessage("null")
	}
	return r
}

// TestFacadeRoutesToolToHub proves the full chain: an MCP client calls a tool,
// the facade forwards it to the hub bus as a capability call, the hub routes it
// to a provider, and the reply flows back as the tool result.
func TestFacadeRoutesToolToHub(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	// Real hub bus.
	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer hub.Close()
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"

	// Provider answering the methods our tools call.
	fakeProvider(t, ctx, busURL, []string{"agents.list", "agents.spawn", "agents.sendMessage"})

	// Facade wired to the hub.
	client := busclient.New(busURL, "")
	go client.Run(ctx)
	server := newServer(client)

	// In-memory MCP client/server pair.
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

	// tools/list should expose our registered tools.
	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("ListTools: %v", err)
	}
	if !hasTool(tools.Tools, "spawn_agent") || !hasTool(tools.Tools, "list_agents") {
		t.Fatalf("expected spawn_agent + list_agents in %v", toolNames(tools.Tools))
	}

	// Call spawn_agent; the provider echoes method+params back through the chain.
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "spawn_agent",
		Arguments: map[string]any{"cwd": "/tmp/x", "model": "claude-opus-4-8"},
	})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	if res.IsError {
		t.Fatalf("tool returned error: %v", textOf(res))
	}
	text := textOf(res)
	if !strings.Contains(text, `"method":"agents.spawn"`) {
		t.Errorf("result did not route to agents.spawn: %s", text)
	}
	if !strings.Contains(text, `"cwd":"/tmp/x"`) {
		t.Errorf("result did not carry params: %s", text)
	}
}

// TestFacadeNoProvider proves a tool call surfaces a clean error (not a hang)
// when no provider has registered the method.
func TestFacadeNoProvider(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer hub.Close()
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"

	client := busclient.New(busURL, "")
	go client.Run(ctx)
	server := newServer(client)

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
		Name:      "list_agents",
		Arguments: map[string]any{},
	})
	if err != nil {
		t.Fatalf("CallTool transport error: %v", err)
	}
	if !res.IsError {
		t.Fatalf("expected IsError when no provider, got: %s", textOf(res))
	}
	if !strings.Contains(textOf(res), "no provider") {
		t.Errorf("expected 'no provider' message, got: %s", textOf(res))
	}
}

func hasTool(tools []*mcp.Tool, name string) bool {
	for _, tl := range tools {
		if tl.Name == name {
			return true
		}
	}
	return false
}

func toolNames(tools []*mcp.Tool) []string {
	out := make([]string, len(tools))
	for i, tl := range tools {
		out[i] = tl.Name
	}
	return out
}

func textOf(res *mcp.CallToolResult) string {
	var b strings.Builder
	for _, c := range res.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			b.WriteString(tc.Text)
		}
	}
	return b.String()
}
