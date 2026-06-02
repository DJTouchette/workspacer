// Package integration holds end-to-end tests that wire the real components
// together. The spine test proves: claudemon SSE -> bridge -> broker -> bus ->
// WebSocket client, with the client none the wiser about the source.
package integration

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/claudemon"
)

// fakeClaudemon serves an SSE stream shaped like claudemon's /events: a
// session.update frame every 20ms until the request is cancelled.
func fakeClaudemon(t *testing.T) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		fl, ok := w.(http.Flusher)
		if !ok {
			t.Error("ResponseWriter is not a Flusher")
			return
		}
		ticker := time.NewTicker(20 * time.Millisecond)
		defer ticker.Stop()
		i := 0
		for {
			select {
			case <-r.Context().Done():
				return
			case <-ticker.C:
				i++
				fmt.Fprintf(w, "event: session.update\ndata: {\"session_id\":\"s%d\",\"event\":\"Stop\",\"state\":{\"mode\":\"input\"}}\n\n", i)
				fl.Flush()
			}
		}
	}))
}

func readFrame(t *testing.T, c *websocket.Conn) bus.Frame {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	_, data, err := c.Read(ctx)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	var f bus.Frame
	if err := json.Unmarshal(data, &f); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	return f
}

func send(t *testing.T, c *websocket.Conn, f bus.Frame) {
	t.Helper()
	data, _ := json.Marshal(f)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := c.Write(ctx, websocket.MessageText, data); err != nil {
		t.Fatalf("write: %v", err)
	}
}

func readUntil(t *testing.T, c *websocket.Conn, op string) bus.Frame {
	t.Helper()
	deadline := time.After(4 * time.Second)
	for {
		select {
		case <-deadline:
			t.Fatalf("timed out waiting for op=%q", op)
		default:
		}
		if f := readFrame(t, c); f.Op == op {
			return f
		}
	}
}

func TestSpineClaudemonToBusClient(t *testing.T) {
	fake := fakeClaudemon(t)
	defer fake.Close()

	b := broker.New()
	busHTTP := httptest.NewServer(bus.NewServer(b).Handler())
	defer busHTTP.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// claudemon -> bridge -> broker
	bridge := claudemon.NewBridge(fake.URL, b)
	go bridge.Run(ctx)

	// A plugin-style client connects to the bus and subscribes to agent.*
	wsURL := strings.Replace(busHTTP.URL, "http://", "ws://", 1) + "/bus"
	dctx, dcancel := context.WithTimeout(ctx, 2*time.Second)
	defer dcancel()
	c, _, err := websocket.Dial(dctx, wsURL, nil)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer c.CloseNow()

	readUntil(t, c, "hello")
	send(t, c, bus.Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	readUntil(t, c, "subscribed")

	// The event originated in (fake) claudemon, crossed the bridge + broker, and
	// arrives here as a normalized agent.* event — the client never sees SSE.
	evFrame := readUntil(t, c, "event")
	if evFrame.Event == nil {
		t.Fatal("event frame had no envelope")
	}
	if evFrame.Event.Type != "agent.state_changed" {
		t.Fatalf("type = %q", evFrame.Event.Type)
	}
	if evFrame.Event.Source != "claudemon" {
		t.Fatalf("source = %q", evFrame.Event.Source)
	}
	if evFrame.Event.ID == "" || evFrame.Event.Time.IsZero() {
		t.Fatalf("event not stamped: %+v", evFrame.Event)
	}
	if !strings.Contains(string(evFrame.Event.Data), `"mode":"input"`) {
		t.Fatalf("payload = %s", evFrame.Event.Data)
	}
}
