package claudemon

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

func TestMapEvent(t *testing.T) {
	data := []byte(`{"session_id":"s1","event":"Stop","state":{"mode":"input","cwd":"/tmp"}}`)
	ev, ok := mapEvent("session.update", data)
	if !ok {
		t.Fatal("expected mapped event")
	}
	if ev.Type != "agent.state_changed" || ev.Source != "claudemon" {
		t.Fatalf("type/source = %q/%q", ev.Type, ev.Source)
	}
	if !strings.Contains(string(ev.Data), `"sessionId":"s1"`) ||
		!strings.Contains(string(ev.Data), `"mode":"input"`) {
		t.Fatalf("payload = %s", ev.Data)
	}
}

func TestMapEventRejectsGarbage(t *testing.T) {
	if _, ok := mapEvent("session.update", []byte(`not json`)); ok {
		t.Error("garbage should not map")
	}
	if _, ok := mapEvent("session.update", []byte(`{"event":"Stop"}`)); ok {
		t.Error("missing session_id should not map")
	}
	if _, ok := mapEvent("other", []byte(`{"session_id":"s1"}`)); ok {
		t.Error("unknown event name should not map")
	}
}

func TestParseSSE(t *testing.T) {
	stream := "event: session.update\n" +
		"data: {\"session_id\":\"s1\"}\n" +
		"\n" +
		": heartbeat\n" +
		"event: session.update\n" +
		"data: {\"session_id\":\"s2\"}\n" +
		"\n"
	var got []string
	err := parseSSE(context.Background(), strings.NewReader(stream), func(name string, data []byte) {
		got = append(got, name+"|"+string(data))
	})
	if err != nil {
		t.Fatalf("parseSSE: %v", err)
	}
	if len(got) != 2 || got[0] != `session.update|{"session_id":"s1"}` || got[1] != `session.update|{"session_id":"s2"}` {
		t.Fatalf("frames = %v", got)
	}
}

// capture is a non-blocking Publisher recording events.
type capture struct{ ch chan event.Envelope }

func (c *capture) Publish(ev event.Envelope) {
	select {
	case c.ch <- ev:
	default:
	}
}

func TestBridgeReconnectsAfterDrop(t *testing.T) {
	// stream() against a dead URL returns an error; Run should keep looping and
	// honor ctx cancellation rather than spin or block.
	b := NewBridge("http://127.0.0.1:1/events", &capture{ch: make(chan event.Envelope, 1)})
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	done := make(chan struct{})
	go func() { b.Run(ctx); close(done) }()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("Run did not exit on ctx cancel")
	}
}
