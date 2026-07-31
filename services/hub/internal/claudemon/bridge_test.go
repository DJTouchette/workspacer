package claudemon

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
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

// A non-2xx is a real failure, not a stream. Without the status check parseSSE
// just read an error body to EOF, so the bridge looked connected while
// republishing nothing — and, because Run discarded the result, said nothing.
func TestBridgeStreamRejectsNon2xx(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.Error(w, "nope", http.StatusNotFound)
	}))
	defer srv.Close()

	b := NewBridge(srv.URL+"/events", &capture{ch: make(chan event.Envelope, 1)})
	err := b.stream(context.Background())
	if err == nil {
		t.Fatal("expected an error for HTTP 404, got nil")
	}
	if !strings.Contains(err.Error(), "404") {
		t.Errorf("error should name the status, got %v", err)
	}
}

// A 2xx stream still parses — the status check must not reject the happy path.
func TestBridgeStreamAcceptsOkStream(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write([]byte("event: session.update\ndata: {\"session_id\":\"s1\"}\n\n"))
	}))
	defer srv.Close()

	got := make(chan event.Envelope, 1)
	b := NewBridge(srv.URL+"/events", &capture{ch: got})
	if err := b.stream(context.Background()); err != nil {
		t.Fatalf("stream: %v", err)
	}
	select {
	case ev := <-got:
		if ev.Type != "agent.state_changed" {
			t.Errorf("republished %q, want agent.state_changed", ev.Type)
		}
	default:
		t.Fatal("nothing republished from a well-formed stream")
	}
}

// The backoff must grow rather than hammering at a flat 1 req/s, and must stay
// bounded. Counting attempts inside a fixed window is the observable proof.
func TestBridgeBacksOffRatherThanSpinning(t *testing.T) {
	var attempts int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		atomic.AddInt32(&attempts, 1)
		http.Error(w, "down", http.StatusServiceUnavailable)
	}))
	defer srv.Close()

	b := NewBridge(srv.URL+"/events", &capture{ch: make(chan event.Envelope, 8)})
	ctx, cancel := context.WithTimeout(context.Background(), 1500*time.Millisecond)
	defer cancel()
	b.Run(ctx)

	// 200,400,800 → ~3-5 attempts in 1.5s. A flat 1s retry would give ~2 and the
	// old code's unconditional loop far more if the server answered instantly.
	n := atomic.LoadInt32(&attempts)
	if n < 2 {
		t.Errorf("only %d attempts — it is not retrying", n)
	}
	if n > 12 {
		t.Errorf("%d attempts in 1.5s — it is spinning, not backing off", n)
	}
}
