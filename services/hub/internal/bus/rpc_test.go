package bus

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
)

// client is a tiny test wrapper around a bus WebSocket connection.
type client struct {
	t  *testing.T
	ws *websocket.Conn
}

func dialClient(t *testing.T, httpURL string) *client {
	return dialClientToken(t, httpURL, "")
}

// dialClientToken connects presenting a ?token= (empty = none), so tests can
// connect as the trusted host or as a capability-scoped plugin.
func dialClientToken(t *testing.T, httpURL, token string) *client {
	t.Helper()
	wsURL := strings.Replace(httpURL, "http://", "ws://", 1) + "/bus"
	if token != "" {
		wsURL += "?token=" + token
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	t.Cleanup(func() { c.CloseNow() })
	cl := &client{t: t, ws: c}
	cl.readUntil("hello")
	return cl
}

func (c *client) send(f Frame) {
	c.t.Helper()
	data, _ := json.Marshal(f)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := c.ws.Write(ctx, websocket.MessageText, data); err != nil {
		c.t.Fatalf("write: %v", err)
	}
}

func (c *client) readUntil(op string) Frame {
	c.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	for {
		_, data, err := c.ws.Read(ctx)
		if err != nil {
			c.t.Fatalf("read (want %q): %v", op, err)
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil {
			c.t.Fatalf("unmarshal: %v", err)
		}
		if f.Op == op {
			return f
		}
	}
}

func rpcServer(t *testing.T) string {
	t.Helper()
	hs := httptest.NewServer(NewServer(broker.New()).Handler())
	t.Cleanup(hs.Close)
	return hs.URL
}

// rpcServerWith returns both the URL and the underlying Server so callers can
// reach internal fields (e.g. router.authorize) from within the same package.
func rpcServerWith(t *testing.T) (string, *Server) {
	t.Helper()
	srv := NewServer(broker.New())
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	return hs.URL, srv
}

// TestCallNotAuthorized pins capability enforcement: a plugin connection (per-
// plugin token) calling a capability it did NOT declare receives an "error"
// frame whose Error contains "not authorized".
func TestCallNotAuthorized(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", []string{"agents.list"}) // not agents.spawn

	caller := dialClientToken(t, url, "plug-tok")
	caller.send(Frame{Op: "call", ID: "auth1", Method: "agents.spawn"})
	e := caller.readUntil("error")
	if e.ID != "auth1" {
		t.Fatalf("correlation id = %q, want auth1", e.ID)
	}
	if !strings.Contains(e.Error, "not authorized") {
		t.Fatalf("error = %q, want it to contain \"not authorized\"", e.Error)
	}
}

// TestPluginMayCallDeclaredCapability is the allow path: a plugin calling a
// capability it declared is routed to the provider normally.
func TestPluginMayCallDeclaredCapability(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", []string{"agents.spawn"})

	provider := dialClientToken(t, url, "host-secret") // trusted host registers the method
	provider.send(Frame{Op: "register", Methods: []string{"agents.spawn"}})
	provider.readUntil("registered")
	go func() {
		f := provider.readUntil("call")
		provider.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
	}()

	caller := dialClientToken(t, url, "plug-tok")
	caller.send(Frame{Op: "call", ID: "c1", Method: "agents.spawn"})
	if r := caller.readUntil("result"); r.ID != "c1" {
		t.Fatalf("correlation id = %q, want c1", r.ID)
	}
}

// TestUnknownTokenRejected: with a host token set, a connection presenting
// neither the host token nor a registered plugin token is refused at handshake.
func TestUnknownTokenRejected(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")

	wsURL := strings.Replace(url, "http://", "ws://", 1) + "/bus?token=bogus"
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if _, _, err := websocket.Dial(ctx, wsURL, nil); err == nil {
		t.Fatal("expected dial with an unknown token to be rejected")
	}
}

func TestCapabilityRoundTrip(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	caller := dialClient(t, url)

	provider.send(Frame{Op: "register", Methods: []string{"agents.spawn"}})
	provider.readUntil("registered")

	// Provider services the forwarded call in the background.
	go func() {
		f := provider.readUntil("call")
		if f.Method != "agents.spawn" {
			return
		}
		var params map[string]string
		_ = json.Unmarshal(f.Params, &params)
		provider.send(Frame{
			Op: "result", ID: f.ID,
			Result: json.RawMessage(`{"sessionId":"sess-for-` + params["cwd"] + `"}`),
		})
	}()

	caller.send(Frame{
		Op: "call", ID: "c1", Method: "agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp/proj"}`),
	})
	res := caller.readUntil("result")
	if res.ID != "c1" {
		t.Fatalf("correlation id = %q want c1", res.ID)
	}
	if !strings.Contains(string(res.Result), "sess-for-/tmp/proj") {
		t.Fatalf("result = %s", res.Result)
	}
}

func TestCallNoProvider(t *testing.T) {
	url := rpcServer(t)
	caller := dialClient(t, url)
	caller.send(Frame{Op: "call", ID: "c1", Method: "nobody.home"})
	e := caller.readUntil("error")
	if e.ID != "c1" || !strings.Contains(e.Error, "no provider") {
		t.Fatalf("error frame = %+v", e)
	}
}

func TestProviderErrorPropagates(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	caller := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"agents.kill"}})
	provider.readUntil("registered")

	go func() {
		f := provider.readUntil("call")
		provider.send(Frame{Op: "error", ID: f.ID, Error: "no such agent"})
	}()

	caller.send(Frame{Op: "call", ID: "k9", Method: "agents.kill"})
	e := caller.readUntil("error")
	if e.ID != "k9" || e.Error != "no such agent" {
		t.Fatalf("error frame = %+v", e)
	}
}

func TestProviderDisconnectFailsPendingCall(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	caller := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"slow.op"}})
	provider.readUntil("registered")

	// Provider receives the call but then drops without replying.
	go func() {
		provider.readUntil("call")
		provider.ws.CloseNow()
	}()

	caller.send(Frame{Op: "call", ID: "x", Method: "slow.op"})
	e := caller.readUntil("error")
	if e.ID != "x" || !strings.Contains(e.Error, "disconnected") {
		t.Fatalf("error frame = %+v", e)
	}
}
