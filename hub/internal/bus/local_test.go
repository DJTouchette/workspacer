package bus

import (
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// newTestServerWith stands up a server and lets the caller register in-process
// capabilities before it starts serving — the path the hub uses for the layout
// document.
func newTestServerWith(t *testing.T, configure func(*Server)) string {
	t.Helper()
	srv := NewServer(broker.New())
	if configure != nil {
		configure(srv)
	}
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	return hs.URL
}

// TestLocalProviderCall proves a WebSocket caller can invoke a RegisterLocal
// capability and get its result back — the hub answering for itself, with no
// remote provider in the picture.
func TestLocalProviderCall(t *testing.T) {
	url := newTestServerWith(t, func(s *Server) {
		s.RegisterLocal("layout.echo", func(params json.RawMessage) (any, error) {
			var in struct {
				N int `json:"n"`
			}
			_ = json.Unmarshal(params, &in)
			return map[string]any{"n": in.N + 1}, nil
		})
	})
	c := dial(t, url)
	readUntil(t, c, "hello")

	send(t, c, Frame{Op: "call", ID: "x1", Method: "layout.echo", Params: json.RawMessage(`{"n":41}`)})
	res := readUntil(t, c, "result")
	if res.ID != "x1" {
		t.Fatalf("result id = %q, want x1", res.ID)
	}
	var out struct {
		N int `json:"n"`
	}
	if err := json.Unmarshal(res.Result, &out); err != nil || out.N != 42 {
		t.Fatalf("result = %s (%v), want {n:42}", res.Result, err)
	}
}

// TestLocalProviderError surfaces a handler error as an error frame to the caller.
func TestLocalProviderError(t *testing.T) {
	url := newTestServerWith(t, func(s *Server) {
		s.RegisterLocal("layout.boom", func(json.RawMessage) (any, error) {
			return nil, errLocalBoom
		})
	})
	c := dial(t, url)
	readUntil(t, c, "hello")

	send(t, c, Frame{Op: "call", ID: "e1", Method: "layout.boom", Params: json.RawMessage(`{}`)})
	res := readUntil(t, c, "error")
	if res.ID != "e1" || !strings.Contains(res.Error, "boom") {
		t.Fatalf("error frame = %+v, want id e1 / contains boom", res)
	}
}

// TestLocalProviderBroadcastReachesOtherClient mirrors the real flow: one client
// calls a local capability that publishes an event; a second, subscribed client
// receives it. This is exactly how a layout.set on one client reaches another.
func TestLocalProviderBroadcastReachesOtherClient(t *testing.T) {
	b := broker.New()
	srv := NewServer(b)
	srv.RegisterLocal("layout.touch", func(json.RawMessage) (any, error) {
		b.Publish(event.New("layout.changed", "hub", map[string]any{"version": 7}))
		return map[string]any{"ok": true}, nil
	})
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)

	// Subscriber connects first and subscribes to the topic.
	watcher := dial(t, hs.URL)
	readUntil(t, watcher, "hello")
	send(t, watcher, Frame{Op: "subscribe", Topics: []string{"layout.changed"}})
	readUntil(t, watcher, "subscribed")

	// Caller triggers the local capability.
	caller := dial(t, hs.URL)
	readUntil(t, caller, "hello")
	send(t, caller, Frame{Op: "call", ID: "t1", Method: "layout.touch", Params: json.RawMessage(`{}`)})
	readUntil(t, caller, "result")

	ev := readUntil(t, watcher, "event")
	if ev.Event == nil || ev.Event.Type != "layout.changed" {
		t.Fatalf("watcher got %+v, want layout.changed", ev.Event)
	}
}

var errLocalBoom = boomError("boom")

type boomError string

func (e boomError) Error() string { return string(e) }
