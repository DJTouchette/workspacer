package main

// Cross-hub facade tests: a REAL two-hub setup (the peer hub linked via
// internal/federation, like federation_test.go composes it), a facade wired to
// the LOCAL hub, and an in-memory MCP client driving the tools. Proves the
// fleet merge, hub-param routing, per-peer failure tolerance, and that the
// bare (hub omitted) behavior is unchanged.

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/federation"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// machineProvider registers methods on a hub bus and answers each call with
// answer(method, params). The stand-in for one machine's Electron main process.
func machineProvider(t *testing.T, ctx context.Context, busURL string, methods []string, answer func(method string, params json.RawMessage) any) {
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
			out, _ := json.Marshal(answer(f.Method, nonNil(f.Params)))
			reply, _ := json.Marshal(busFrame{Op: "result", ID: f.ID, Result: out})
			_ = conn.Write(ctx, websocket.MessageText, reply)
		}
	}()
}

// echoAnswer builds the marker-echo shape both machines' providers use for
// per-session methods, so a test can tell WHICH hub answered and see the exact
// params the provider received.
func echoAnswer(machine string) func(method string, params json.RawMessage) any {
	return func(method string, params json.RawMessage) any {
		return map[string]any{"machine": machine, "method": method, "params": params}
	}
}

// newFederatedLocalHub stands up the LOCAL hub linked to the given peers,
// mirroring cmd/hub/main.go's wiring: SetFederation + the hub-local
// federation.peers method. Returns the local bus URL and the manager (for
// liveness polling).
func newFederatedLocalHub(t *testing.T, ctx context.Context, peers []federation.Peer) (string, *federation.Manager) {
	t.Helper()
	localBroker := broker.New()
	localBus := bus.NewServer(localBroker)
	fed, err := federation.New(localBroker, peers)
	if err != nil {
		t.Fatalf("federation.New: %v", err)
	}
	localBus.SetFederation(fed)
	localBus.RegisterLocal("federation.peers", func(json.RawMessage) (any, error) {
		return fed.PeersInfo(), nil
	})
	go fed.Run(ctx)
	srv := httptest.NewServer(localBus.Handler())
	t.Cleanup(srv.Close)
	return strings.Replace(srv.URL, "http", "ws", 1) + "/bus", fed
}

// waitPeerConnected polls until the named federation link is up.
func waitPeerConnected(t *testing.T, fed *federation.Manager, name string) {
	t.Helper()
	deadline := time.Now().Add(8 * time.Second)
	for time.Now().Before(deadline) {
		for _, p := range fed.PeersInfo() {
			if p.Name == name && p.Connected {
				return
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatalf("peer %q never connected", name)
}

// connectFacade wires a facade server (for one tier) to a hub bus and returns
// an in-memory MCP client session driving it.
func connectFacade(t *testing.T, ctx context.Context, busURL string, scope authtoken.Scope) *mcp.ClientSession {
	t.Helper()
	client := busclient.New(busURL, "")
	go client.Run(ctx)
	server := newServer(client, scope)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	t.Cleanup(func() { cs.Close() })
	return cs
}

func callTool(t *testing.T, ctx context.Context, cs *mcp.ClientSession, name string, args map[string]any) *mcp.CallToolResult {
	t.Helper()
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: name, Arguments: args})
	if err != nil {
		t.Fatalf("CallTool %s: %v", name, err)
	}
	return res
}

// TestFleetToolsMergeAcrossHubs proves the fleet-wide facade over a real
// two-hub federation: list tools merge and tag remote rows, per-session tools
// route by the hub param (with the param stripped before the peer sees it),
// spawn_agent routes to the peer, and the bare form stays local.
func TestFleetToolsMergeAcrossHubs(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	// The peer hub ("work machine") with a full provider.
	peerSrv := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer peerSrv.Close()
	peerURL := strings.Replace(peerSrv.URL, "http", "ws", 1) + "/bus"
	peerEcho := echoAnswer("peer")
	machineProvider(t, ctx, peerURL,
		[]string{"agents.list", "sessions.snapshots", "sessions.snapshot", "sessions.transcript", "agents.spawn", "agents.sendMessage"},
		func(method string, params json.RawMessage) any {
			switch method {
			case "agents.list":
				return []map[string]any{{"sessionId": "remote-1", "state": "working"}}
			case "sessions.snapshots":
				return []map[string]any{{"sessionId": "remote-1", "detail": true}}
			}
			return peerEcho(method, params)
		})

	// The local hub, federated to the peer, with its own provider.
	localURL, fed := newFederatedLocalHub(t, ctx, []federation.Peer{{Name: "work", URL: peerURL}})
	localEcho := echoAnswer("local")
	machineProvider(t, ctx, localURL,
		[]string{"agents.list", "sessions.snapshots", "sessions.snapshot", "sessions.transcript", "agents.spawn"},
		func(method string, params json.RawMessage) any {
			switch method {
			case "agents.list":
				return []map[string]any{{"sessionId": "local-1", "state": "idle"}}
			case "sessions.snapshots":
				return []map[string]any{{"sessionId": "local-1", "detail": true}}
			}
			return localEcho(method, params)
		})
	waitPeerConnected(t, fed, "work")

	cs := connectFacade(t, ctx, localURL, authtoken.ScopeOperator)

	// ── list tools merge; remote rows tagged, local rows untagged ──────────
	for _, tool := range []string{"list_agents", "list_snapshots"} {
		res := callTool(t, ctx, cs, tool, map[string]any{})
		if res.IsError {
			t.Fatalf("%s errored: %s", tool, textOf(res))
		}
		var rows []map[string]any
		if err := json.Unmarshal([]byte(textOf(res)), &rows); err != nil {
			t.Fatalf("%s result is not a JSON array: %v (%s)", tool, err, textOf(res))
		}
		if len(rows) != 2 {
			t.Fatalf("%s: want 2 merged rows, got %d: %s", tool, len(rows), textOf(res))
		}
		byID := map[string]map[string]any{}
		for _, r := range rows {
			byID[r["sessionId"].(string)] = r
		}
		if _, tagged := byID["local-1"]["hub"]; tagged {
			t.Errorf("%s: local row must stay untagged: %v", tool, byID["local-1"])
		}
		if hub := byID["remote-1"]["hub"]; hub != "work" {
			t.Errorf("%s: remote row hub = %v, want \"work\"", tool, hub)
		}
	}

	// ── hub param routes per-session calls to the peer, and is stripped ────
	res := callTool(t, ctx, cs, "get_snapshot", map[string]any{"sessionId": "remote-1", "hub": "work"})
	text := textOf(res)
	if res.IsError || !strings.Contains(text, `"machine":"peer"`) || !strings.Contains(text, `"method":"sessions.snapshot"`) {
		t.Fatalf("get_snapshot with hub did not reach the peer: %s", text)
	}
	if strings.Contains(text, `"hub"`) {
		t.Errorf("routing param leaked into the forwarded params: %s", text)
	}
	if !strings.Contains(text, `"sessionId":"remote-1"`) {
		t.Errorf("real params were not forwarded: %s", text)
	}

	// Bare form unchanged: no hub → the local provider answers.
	res = callTool(t, ctx, cs, "get_snapshot", map[string]any{"sessionId": "local-1"})
	if !strings.Contains(textOf(res), `"machine":"local"`) {
		t.Fatalf("bare get_snapshot should stay local: %s", textOf(res))
	}

	// spawn_agent's hub routes to the peer's agents.spawn.
	res = callTool(t, ctx, cs, "spawn_agent", map[string]any{"cwd": "/tmp/x", "hub": "work"})
	text = textOf(res)
	if res.IsError || !strings.Contains(text, `"machine":"peer"`) || !strings.Contains(text, `"method":"agents.spawn"`) {
		t.Fatalf("spawn_agent with hub did not reach the peer: %s", text)
	}
	if strings.Contains(text, `"hub"`) {
		t.Errorf("spawn hub param leaked into the forwarded params: %s", text)
	}

	// ── the hub input rides the schema (embedded hubArg flattens) ──────────
	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("ListTools: %v", err)
	}
	for _, want := range []string{"get_snapshot", "spawn_agent", "send_message"} {
		found := false
		for _, tl := range tools.Tools {
			if tl.Name != want {
				continue
			}
			found = true
			schema, _ := json.Marshal(tl.InputSchema)
			if !strings.Contains(string(schema), `"hub"`) {
				t.Errorf("%s input schema lacks the hub property: %s", want, schema)
			}
		}
		if !found {
			t.Errorf("tool %s not registered", want)
		}
	}

	// ── tier correctness: a view-tier facade's hub-qualified read forwards ──
	// (the facade's bus connection is trusted; the tool exists in view because
	// the BARE method does — same gate as before).
	viewCS := connectFacade(t, ctx, localURL, authtoken.ScopeView)
	res = callTool(t, ctx, viewCS, "get_transcript", map[string]any{"sessionId": "remote-1", "hub": "work"})
	if res.IsError || !strings.Contains(textOf(res), `"machine":"peer"`) {
		t.Fatalf("view-tier get_transcript with hub should forward: %s", textOf(res))
	}
}

// TestFleetMergeToleratesPeerFailures pins the failure budget: a configured but
// unreachable peer, and a connected peer that cannot answer a method, each cost
// only their own rows — never the call.
func TestFleetMergeToleratesPeerFailures(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	// The peer answers agents.list but has NO provider for sessions.snapshots.
	peerSrv := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer peerSrv.Close()
	peerURL := strings.Replace(peerSrv.URL, "http", "ws", 1) + "/bus"
	machineProvider(t, ctx, peerURL, []string{"agents.list"},
		func(method string, params json.RawMessage) any {
			return []map[string]any{{"sessionId": "remote-1"}}
		})

	// Local hub federated to the live peer AND a dead one (nothing listens
	// there — its link never connects, so federation.peers reports it
	// connected:false and the merge skips it without waiting).
	localURL, fed := newFederatedLocalHub(t, ctx, []federation.Peer{
		{Name: "work", URL: peerURL},
		{Name: "ghost", URL: "ws://127.0.0.1:1/bus"},
	})
	machineProvider(t, ctx, localURL, []string{"agents.list", "sessions.snapshots"},
		func(method string, params json.RawMessage) any {
			return []map[string]any{{"sessionId": "local-1"}}
		})
	waitPeerConnected(t, fed, "work")

	cs := connectFacade(t, ctx, localURL, authtoken.ScopeOperator)

	// Both live hubs contribute; the dead peer costs nothing.
	res := callTool(t, ctx, cs, "list_agents", map[string]any{})
	if res.IsError {
		t.Fatalf("list_agents errored: %s", textOf(res))
	}
	if text := textOf(res); !strings.Contains(text, "local-1") || !strings.Contains(text, "remote-1") {
		t.Fatalf("list_agents missing rows: %s", text)
	}

	// The peer's per-method failure ("no provider for sessions.snapshots")
	// costs its rows only — local rows still come back, and it is not an error.
	res = callTool(t, ctx, cs, "list_snapshots", map[string]any{})
	if res.IsError {
		t.Fatalf("list_snapshots must tolerate the peer's missing provider: %s", textOf(res))
	}
	var rows []map[string]any
	if err := json.Unmarshal([]byte(textOf(res)), &rows); err != nil {
		t.Fatalf("list_snapshots result not an array: %v (%s)", err, textOf(res))
	}
	if len(rows) != 1 || rows[0]["sessionId"] != "local-1" {
		t.Fatalf("list_snapshots should hold exactly the local row: %s", textOf(res))
	}
}

// TestFleetToolsWithoutFederation pins back-compat on a hub with no federation
// configured: the listing tools answer with the local result untouched (the
// federation.peers probe fails and merges nothing), including non-array
// results, which pass through verbatim.
func TestFleetToolsWithoutFederation(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer hub.Close()
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	machineProvider(t, ctx, busURL, []string{"agents.list", "sessions.snapshots"},
		func(method string, params json.RawMessage) any {
			if method == "agents.list" {
				return []map[string]any{{"sessionId": "local-1"}}
			}
			return map[string]any{"not": "an array"}
		})

	cs := connectFacade(t, ctx, busURL, authtoken.ScopeOperator)

	res := callTool(t, ctx, cs, "list_agents", map[string]any{})
	if res.IsError {
		t.Fatalf("list_agents errored without federation: %s", textOf(res))
	}
	var rows []map[string]any
	if err := json.Unmarshal([]byte(textOf(res)), &rows); err != nil || len(rows) != 1 {
		t.Fatalf("list_agents should return the local rows unchanged: %s", textOf(res))
	}
	if _, tagged := rows[0]["hub"]; tagged {
		t.Errorf("local row gained a hub tag: %s", textOf(res))
	}

	// A non-array local result is passed through byte-for-byte.
	res = callTool(t, ctx, cs, "list_snapshots", map[string]any{})
	if res.IsError || !strings.Contains(textOf(res), `"not":"an array"`) {
		t.Fatalf("non-array local result should pass through verbatim: %s", textOf(res))
	}
}
