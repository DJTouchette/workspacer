package federation

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
	"github.com/djtouchette/workspacer-hub/internal/event"
)

func TestParsePeerFlag(t *testing.T) {
	p, err := ParsePeerFlag("name=work,url=ws://host:7895/bus,token=abc")
	if err != nil || p.Name != "work" || p.URL != "ws://host:7895/bus" || p.Token != "abc" {
		t.Fatalf("parse: %+v %v", p, err)
	}
	// Order-insensitive, token optional.
	if p, err = ParsePeerFlag("url=wss://x/bus,name=a-b_2"); err != nil || p.Name != "a-b_2" {
		t.Fatalf("parse 2: %+v %v", p, err)
	}
	for _, bad := range []string{
		"",                                  // empty
		"name=work",                         // no url
		"url=ws://x/bus",                    // no name
		"name=wo/rk,url=ws://x/bus",         // slash in name (qualification syntax)
		"name=wo:rk,url=ws://x/bus",         // colon in name
		"name=work,url=http://x/bus",        // not a ws url
		"name=work,url=ws://x/bus,zork=zap", // unknown key
	} {
		if _, err := ParsePeerFlag(bad); err == nil {
			t.Errorf("ParsePeerFlag(%q) should fail", bad)
		}
	}
}

// wsFrame is the raw wire shape for the test's hand-rolled clients.
type wsFrame struct {
	Op      string          `json:"op"`
	ID      string          `json:"id,omitempty"`
	Method  string          `json:"method,omitempty"`
	Methods []string        `json:"methods,omitempty"`
	Topics  []string        `json:"topics,omitempty"`
	Params  json.RawMessage `json:"params,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   string          `json:"error,omitempty"`
	Event   *event.Envelope `json:"event,omitempty"`
}

// dial opens a raw websocket client to a bus URL.
func dial(t *testing.T, ctx context.Context, busURL string) *websocket.Conn {
	t.Helper()
	conn, _, err := websocket.Dial(ctx, busURL, nil)
	if err != nil {
		t.Fatalf("dial %s: %v", busURL, err)
	}
	conn.SetReadLimit(8 << 20)
	return conn
}

func send(t *testing.T, ctx context.Context, c *websocket.Conn, f wsFrame) {
	t.Helper()
	b, _ := json.Marshal(f)
	if err := c.Write(ctx, websocket.MessageText, b); err != nil {
		t.Fatalf("write: %v", err)
	}
}

// TestFederationEndToEnd is the Phase 2+3 done-when, against two REAL hubs:
// a peer event arrives locally stamped with the peer name; a qualified call
// reaches the peer's provider; the tier check runs against the bare method;
// unknown peers and off-list topics are refused; the tree invariant drops
// pre-stamped events.
func TestFederationEndToEnd(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()

	// ── the PEER hub (the "work machine") ──────────────────────────────────
	peerSrv := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer peerSrv.Close()
	peerURL := strings.Replace(peerSrv.URL, "http", "ws", 1) + "/bus"

	// A provider on the peer answering agents.list.
	prov := dial(t, ctx, peerURL)
	defer prov.CloseNow()
	send(t, ctx, prov, wsFrame{Op: "register", Methods: []string{"agents.list"}})
	go func() {
		for {
			_, data, err := prov.Read(ctx)
			if err != nil {
				return
			}
			var f wsFrame
			if json.Unmarshal(data, &f) != nil || f.Op != "call" {
				continue
			}
			res, _ := json.Marshal(map[string]any{"agents": []string{"peer-agent-1"}, "method": f.Method})
			send(t, ctx, prov, wsFrame{Op: "result", ID: f.ID, Result: res})
		}
	}()

	// ── the LOCAL hub, federated to the peer ───────────────────────────────
	localBroker := broker.New()
	localBus := bus.NewServer(localBroker)
	// Scoped tokens on the local hub, so the tier check is provable.
	viewTok := "view-token-1234"
	localBus.SetScopedTokenLookup(func(tok string) (bus.ScopedIdent, bool) {
		if tok == viewTok {
			return bus.ScopedIdent{Scope: "view", Methods: authtoken.ScopeView.Methods()}, true
		}
		return bus.ScopedIdent{}, false
	})
	fed, err := New(localBroker, []Peer{{Name: "work", URL: peerURL}})
	if err != nil {
		t.Fatal(err)
	}
	localBus.SetFederation(fed)
	go fed.Run(ctx)
	localSrv := httptest.NewServer(localBus.Handler())
	defer localSrv.Close()
	localURL := strings.Replace(localSrv.URL, "http", "ws", 1) + "/bus"

	// A local subscriber standing in for the desktop's hub client.
	subEvents := make(chan event.Envelope, 32)
	sub := dial(t, ctx, localURL)
	defer sub.CloseNow()
	send(t, ctx, sub, wsFrame{Op: "subscribe", Topics: []string{"agent.*", "layout.*", "hub.peer.*"}})
	go func() {
		for {
			_, data, err := sub.Read(ctx)
			if err != nil {
				return
			}
			var f wsFrame
			if json.Unmarshal(data, &f) == nil && f.Op == "event" && f.Event != nil {
				subEvents <- *f.Event
			}
		}
	}()

	next := func() event.Envelope {
		t.Helper()
		select {
		case ev := <-subEvents:
			return ev
		case <-time.After(8 * time.Second):
			t.Fatal("no event arrived on the local bus")
			return event.Envelope{}
		}
	}

	// The federation link connects with backoff; hub.peer.connected announces it.
	ev := next()
	if ev.Type != "hub.peer.connected" || !strings.Contains(string(ev.Data), `"peer":"work"`) {
		t.Fatalf("expected hub.peer.connected first, got %+v", ev)
	}

	// ── events federate, stamped ───────────────────────────────────────────
	peerPub := dial(t, ctx, peerURL)
	defer peerPub.CloseNow()
	send(t, ctx, peerPub, wsFrame{Op: "publish", Event: &event.Envelope{
		Type: "agent.snapshot", Source: "test", Data: json.RawMessage(`{"sessionId":"remote-1"}`),
	}})
	ev = next()
	if ev.Type != "agent.snapshot" || ev.Hub != "work" {
		t.Fatalf("federated event wrong: type=%q hub=%q", ev.Type, ev.Hub)
	}
	if string(ev.Data) != `{"sessionId":"remote-1"}` {
		t.Fatalf("payload rewritten: %s", ev.Data)
	}

	// Tree invariant: a peer event ALREADY carrying a Hub stamp is dropped.
	// Curated list: a topic outside ForwardTopics (layout.changed) never
	// federates. Prove both by absence: publish them, then a sentinel, and
	// require the sentinel to be the next thing the subscriber sees.
	send(t, ctx, peerPub, wsFrame{Op: "publish", Event: &event.Envelope{
		Type: "agent.snapshot", Source: "test", Hub: "upstream", Data: json.RawMessage(`{"nested":true}`),
	}})
	send(t, ctx, peerPub, wsFrame{Op: "publish", Event: &event.Envelope{
		Type: "layout.changed", Source: "test", Data: json.RawMessage(`{"forged":"layout"}`),
	}})
	send(t, ctx, peerPub, wsFrame{Op: "publish", Event: &event.Envelope{
		Type: "agent.state_changed", Source: "test", Data: json.RawMessage(`{"sentinel":1}`),
	}})
	ev = next()
	if ev.Type != "agent.state_changed" || !strings.Contains(string(ev.Data), "sentinel") {
		t.Fatalf("dropped-event leak: got %q (%s) before the sentinel", ev.Type, ev.Data)
	}

	// ── qualified calls route to the peer ──────────────────────────────────
	call := func(c *websocket.Conn, id, method string) wsFrame {
		t.Helper()
		send(t, ctx, c, wsFrame{Op: "call", ID: id, Method: method, Params: json.RawMessage(`{}`)})
		deadline := time.Now().Add(8 * time.Second)
		for time.Now().Before(deadline) {
			_, data, err := c.Read(ctx)
			if err != nil {
				t.Fatalf("read reply: %v", err)
			}
			var f wsFrame
			if json.Unmarshal(data, &f) == nil && (f.Op == "result" || f.Op == "error") && f.ID == id {
				return f
			}
		}
		t.Fatalf("no reply to %s", method)
		return wsFrame{}
	}

	caller := dial(t, ctx, localURL)
	defer caller.CloseNow()
	res := call(caller, "c1", "hub:work/agents.list")
	if res.Op != "result" || !strings.Contains(string(res.Result), "peer-agent-1") {
		t.Fatalf("qualified call failed: %+v", res)
	}

	// Unknown peer: refused, not treated as a literal method.
	res = call(caller, "c2", "hub:nope/agents.list")
	if res.Op != "error" || !strings.Contains(res.Error, "unknown federation peer") {
		t.Fatalf("unknown peer not refused: %+v", res)
	}

	// ── tier check against the BARE method ─────────────────────────────────
	viewCaller := dial(t, ctx, localURL+"?token="+viewTok)
	defer viewCaller.CloseNow()
	res = call(viewCaller, "v1", "hub:work/agents.list")
	if res.Op != "result" {
		t.Fatalf("view token should reach hub:work/agents.list: %+v", res)
	}
	res = call(viewCaller, "v2", "hub:work/agents.spawn")
	if res.Op != "error" {
		t.Fatalf("view token must NOT reach hub:work/agents.spawn: %+v", res)
	}
}
