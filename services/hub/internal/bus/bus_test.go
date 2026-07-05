package bus

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

func newTestServer(t *testing.T) string {
	t.Helper()
	srv := NewServer(broker.New())
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	return hs.URL
}

func dial(t *testing.T, httpURL string) *websocket.Conn {
	t.Helper()
	wsURL := strings.Replace(httpURL, "http://", "ws://", 1) + "/bus"
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	t.Cleanup(func() { c.CloseNow() })
	return c
}

func send(t *testing.T, c *websocket.Conn, f Frame) {
	t.Helper()
	data, _ := json.Marshal(f)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := c.Write(ctx, websocket.MessageText, data); err != nil {
		t.Fatalf("write: %v", err)
	}
}

// readUntil reads frames until one with op==want arrives (or times out).
func readUntil(t *testing.T, c *websocket.Conn, want string) Frame {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	for {
		_, data, err := c.Read(ctx)
		if err != nil {
			t.Fatalf("read (waiting for %q): %v", want, err)
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil {
			t.Fatalf("unmarshal: %v", err)
		}
		if f.Op == want {
			return f
		}
	}
}

// expectNoEvent fails if any "event" frame arrives within the window.
func expectNoEvent(t *testing.T, c *websocket.Conn) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()
	for {
		_, data, err := c.Read(ctx)
		if err != nil {
			return // timeout = success
		}
		var f Frame
		_ = json.Unmarshal(data, &f)
		if f.Op == "event" {
			t.Fatalf("unexpected event: %+v", f.Event)
		}
	}
}

func TestEndToEndPublishSubscribe(t *testing.T) {
	url := newTestServer(t)
	sub := dial(t, url)
	pub := dial(t, url)

	readUntil(t, sub, "hello")
	readUntil(t, pub, "hello")

	// Subscribe and wait for the ack so the registration is live before we publish.
	send(t, sub, Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	readUntil(t, sub, "subscribed")

	send(t, pub, Frame{Op: "publish", Event: &event.Envelope{
		Type:   "agent.spawned",
		Source: "test",
		Data:   json.RawMessage(`{"id":"a1"}`),
	}})

	got := readUntil(t, sub, "event")
	if got.Event == nil || got.Event.Type != "agent.spawned" {
		t.Fatalf("got %+v", got.Event)
	}
	if got.Event.ID == "" || got.Event.Time.IsZero() {
		t.Fatalf("server should stamp id+time: %+v", got.Event)
	}
	if string(got.Event.Data) != `{"id":"a1"}` {
		t.Fatalf("payload mangled: %s", got.Event.Data)
	}
}

func TestSubscriptionFilters(t *testing.T) {
	url := newTestServer(t)
	sub := dial(t, url)
	pub := dial(t, url)
	readUntil(t, sub, "hello")
	readUntil(t, pub, "hello")

	send(t, sub, Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	readUntil(t, sub, "subscribed")

	// Different namespace — must not be delivered.
	send(t, pub, Frame{Op: "publish", Event: &event.Envelope{Type: "git.changed"}})
	expectNoEvent(t, sub)
}

func TestHealth(t *testing.T) {
	url := newTestServer(t)
	resp, err := http.Get(url + "/health")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("status %d", resp.StatusCode)
	}
	var body map[string]any
	if err := json.NewDecoder(resp.Body).Decode(&body); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if body["status"] != "ok" {
		t.Fatalf("body=%v", body)
	}
	// No token configured (the loopback default): detailed counts are exposed.
	if _, ok := body["subscribers"]; !ok {
		t.Fatalf("expected subscriber/method counts with no token configured: %v", body)
	}
}

// getHealth fetches /health, optionally with a bearer token, and returns the
// decoded body. Uses a fresh server so SetToken is applied before any request.
func getHealth(t *testing.T, srv *Server, token string) map[string]any {
	t.Helper()
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	req, err := http.NewRequest(http.MethodGet, hs.URL+"/health", nil)
	if err != nil {
		t.Fatalf("new request: %v", err)
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("status %d", resp.StatusCode)
	}
	var body map[string]any
	if err := json.NewDecoder(resp.Body).Decode(&body); err != nil {
		t.Fatalf("decode: %v", err)
	}
	return body
}

// TestHealthGating verifies SECURITY.md #4: when a token is configured, an
// unauthenticated /health probe gets liveness only (no subscriber/method
// counts), while an authorized caller still gets the detailed counts.
func TestHealthGating(t *testing.T) {
	t.Run("unauthenticated caller gets status only when a token is set", func(t *testing.T) {
		srv := NewServer(broker.New())
		srv.SetToken("secret")
		body := getHealth(t, srv, "")
		if body["status"] != "ok" {
			t.Fatalf("status=%v", body["status"])
		}
		if _, ok := body["subscribers"]; ok {
			t.Fatalf("subscriber count leaked to unauthenticated caller: %v", body)
		}
		if _, ok := body["methods"]; ok {
			t.Fatalf("method count leaked to unauthenticated caller: %v", body)
		}
	})

	t.Run("authorized caller still sees the detailed counts", func(t *testing.T) {
		srv := NewServer(broker.New())
		srv.SetToken("secret")
		body := getHealth(t, srv, "secret")
		if _, ok := body["subscribers"]; !ok {
			t.Fatalf("expected subscriber count for authorized caller: %v", body)
		}
		if _, ok := body["methods"]; !ok {
			t.Fatalf("expected method count for authorized caller: %v", body)
		}
	})
}
