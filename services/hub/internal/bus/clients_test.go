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

// serve starts an already-configured server. The sibling helpers build their
// own server, and these tests need the handle back to read Clients() off it.
func serve(t *testing.T, srv *Server) string {
	t.Helper()
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	return hs.URL
}

// dialRaw dials /bus with an arbitrary query string appended.
func dialRaw(t *testing.T, httpURL, query string) *websocket.Conn {
	t.Helper()
	wsURL := strings.Replace(httpURL, "http://", "ws://", 1) + "/bus" + query
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	t.Cleanup(func() { c.CloseNow() })
	return c
}

func clientsOf(t *testing.T, srv *Server) []ClientInfo {
	t.Helper()
	return srv.Clients()
}

// A plain connection is a user-facing client, and its activity clock starts at
// the connect rather than at zero — somebody who just opened the app IS using
// the machine, even before they have done anything with it.
func TestClientsReportsALiveConnection(t *testing.T) {
	srv := NewServer(broker.New())
	url := serve(t, srv)
	c := dial(t, url)
	readUntil(t, c, "hello")

	got := clientsOf(t, srv)
	if len(got) != 1 {
		t.Fatalf("want 1 connection, got %d (%+v)", len(got), got)
	}
	if !got[0].UserFacing() {
		t.Errorf("a plain client is not infrastructure: %+v", got[0])
	}
	if time.Since(got[0].LastActive) > time.Minute {
		t.Errorf("LastActive = %v, want ~now (a connection that just opened is in use)", got[0].LastActive)
	}
	if got[0].Label == "" {
		t.Error("no label — a blocker naming an unlabelled connection tells an operator nothing")
	}
}

// A capability provider is infrastructure. It is connected because the machine
// is running, not because a person is at it.
func TestClientsMarksProvidersAsInfrastructure(t *testing.T) {
	srv := NewServer(broker.New())
	url := serve(t, srv)
	c := dial(t, url)
	readUntil(t, c, "hello")
	send(t, c, Frame{Op: "register", Methods: []string{"agents.list"}})
	readUntil(t, c, "registered")

	got := clientsOf(t, srv)
	if len(got) != 1 {
		t.Fatalf("want 1 connection, got %d", len(got))
	}
	if !got[0].Provider {
		t.Fatalf("a registered provider was not marked as one: %+v", got[0])
	}
	if got[0].UserFacing() {
		t.Error("a provider must not read as somebody using the machine")
	}
}

// The hub's own loopback client. Without this the hub asking itself whether
// anything is happening would find its own question and answer yes forever.
func TestClientsMarksTheHubsOwnLoopbackClient(t *testing.T) {
	srv := NewServer(broker.New())
	srv.SetInternalKey("s3cret-nonce")
	url := serve(t, srv)

	c := dialRaw(t, url, "?"+internalDialParam+"=s3cret-nonce")
	readUntil(t, c, "hello")
	other := dialRaw(t, url, "?"+internalDialParam+"=not-the-key")
	readUntil(t, other, "hello")

	got := clientsOf(t, srv)
	if len(got) != 2 {
		t.Fatalf("want 2 connections, got %d", len(got))
	}
	if !got[0].Internal || got[0].UserFacing() {
		t.Errorf("the self-client was not recognised: %+v", got[0])
	}
	if got[1].Internal {
		t.Errorf("a wrong key must not pass as internal: %+v", got[1])
	}
}

// The distinction the whole client rule turns on. A `call` moves the clock; a
// `subscribe` does not, because that is what a client says on the way IN, and
// a reconnecting background tab would otherwise look like continuous use.
func TestOnlyCallsAndPublishesCountAsActivity(t *testing.T) {
	srv := NewServer(broker.New())
	srv.RegisterLocal("layout.get", func(json.RawMessage) (any, error) { return map[string]any{}, nil })
	url := serve(t, srv)
	c := dial(t, url)
	readUntil(t, c, "hello")

	// Rewind the clock so a fresh frame is visibly different from the connect.
	rewind := time.Now().Add(-time.Hour)
	for _, cn := range liveConns(srv) {
		cn.lastActiveMilli.Store(rewind.UnixMilli())
	}

	send(t, c, Frame{Op: "subscribe", Topics: []string{"agent.snapshot"}})
	readUntil(t, c, "subscribed")
	if got := clientsOf(t, srv); got[0].LastActive.After(rewind.Add(time.Second)) {
		t.Fatalf("a subscribe moved the activity clock (%v) — a background tab that reconnects forever would then read as permanent use", got[0].LastActive)
	}

	send(t, c, Frame{Op: "call", ID: "c1", Method: "layout.get"})
	readUntil(t, c, "result")
	if got := clientsOf(t, srv); !got[0].LastActive.After(rewind.Add(time.Second)) {
		t.Fatalf("a call did NOT move the activity clock (%v)", got[0].LastActive)
	}
}

// A handler must be able to tell which connection is asking, so it can exclude
// the caller: whoever asks "is anything using this machine" is itself
// something using this machine.
func TestCallerIdentityCarriesTheConnectionID(t *testing.T) {
	seen := make(chan uint64, 1)
	srv := NewServer(broker.New())
	srv.RegisterLocalIdent("layout.whoami", func(c CallerIdentity, _ json.RawMessage) (any, error) {
		seen <- c.ConnID
		return map[string]any{}, nil
	})
	url := serve(t, srv)
	c := dial(t, url)
	readUntil(t, c, "hello")
	send(t, c, Frame{Op: "call", ID: "w1", Method: "layout.whoami"})
	readUntil(t, c, "result")

	var connID uint64
	select {
	case connID = <-seen:
	case <-time.After(2 * time.Second):
		t.Fatal("handler never ran")
	}
	if connID == 0 {
		t.Fatal("ConnID was zero — the handler cannot exclude its own caller")
	}
	got := clientsOf(t, srv)
	if len(got) != 1 || got[0].ConnID != connID {
		t.Fatalf("ConnID %d does not match the live connection %+v", connID, got)
	}
}

func TestInternalDialURL(t *testing.T) {
	for _, tc := range []struct{ in, key, want string }{
		{"ws://127.0.0.1:7895/bus", "k", "ws://127.0.0.1:7895/bus?internal=k"},
		{"ws://127.0.0.1:7895/bus?token=t", "k", "ws://127.0.0.1:7895/bus?token=t&internal=k"},
		{"ws://127.0.0.1:7895/bus", "", "ws://127.0.0.1:7895/bus"},
	} {
		if got := InternalDialURL(tc.in, tc.key); got != tc.want {
			t.Errorf("InternalDialURL(%q, %q) = %q, want %q", tc.in, tc.key, got, tc.want)
		}
	}
}

// liveConns reaches into the router for the test above. Package-internal.
func liveConns(s *Server) []*conn {
	s.router.mu.Lock()
	defer s.router.mu.Unlock()
	out := make([]*conn, 0, len(s.router.conns))
	for _, cn := range s.router.conns {
		out = append(out, cn)
	}
	return out
}
