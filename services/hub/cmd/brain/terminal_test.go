package main

import (
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"
)

func TestTerminalForwarderRepublishes(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("/sessions/s1/stream", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		// First frame is the ring-buffer replay, then a live chunk.
		io.WriteString(w, "event: pty.bytes\ndata: "+base64.StdEncoding.EncodeToString([]byte("hello "))+"\n\n")
		io.WriteString(w, "event: pty.bytes\ndata: "+base64.StdEncoding.EncodeToString([]byte("world"))+"\n\n")
		if fl, ok := w.(http.Flusher); ok {
			fl.Flush()
		}
		<-r.Context().Done()
	})
	srv := httptest.NewServer(mux)
	defer srv.Close()

	var mu sync.Mutex
	var topic string
	var got []byte
	done := make(chan struct{})
	publish := func(eventType string, data json.RawMessage) {
		mu.Lock()
		defer mu.Unlock()
		topic = eventType
		var b64 string
		_ = json.Unmarshal(data, &b64)
		dec, _ := base64.StdEncoding.DecodeString(b64)
		got = append(got, dec...)
		// Both chunks coalesce into "hello world".
		if string(got) == "hello world" {
			select {
			case <-done:
			default:
				close(done)
			}
		}
	}

	hub := newTerminalHub(newClaudemonClient(srv.URL), publish)
	if hub.keepalive("s1") {
		t.Fatal("keepalive should be false before attach")
	}
	hub.attach("s1")
	defer hub.detach("s1")

	select {
	case <-done:
	case <-time.After(3 * time.Second):
		mu.Lock()
		defer mu.Unlock()
		t.Fatalf("did not receive coalesced bytes; got %q on topic %q", string(got), topic)
	}

	mu.Lock()
	if topic != "pty.bytes.s1" {
		t.Errorf("topic = %q, want pty.bytes.s1", topic)
	}
	mu.Unlock()

	if !hub.keepalive("s1") {
		t.Error("keepalive should be true while attached")
	}
}

func TestTerminalLeaseSweep(t *testing.T) {
	hub := newTerminalHub(newClaudemonClient("http://unused"), func(string, json.RawMessage) {})
	hub.attach("s1")

	// A live lease survives a sweep...
	hub.sweepExpired(time.Now())
	hub.mu.Lock()
	n := len(hub.fwd)
	hub.mu.Unlock()
	if n != 1 {
		t.Fatalf("live lease should survive a sweep, got %d forwarders", n)
	}

	// ...but a lapsed one is detached.
	hub.mu.Lock()
	hub.fwd["s1"].mu.Lock()
	hub.fwd["s1"].deadline = time.Now().Add(-time.Second)
	hub.fwd["s1"].mu.Unlock()
	hub.mu.Unlock()

	hub.sweepExpired(time.Now())
	hub.mu.Lock()
	n = len(hub.fwd)
	hub.mu.Unlock()
	if n != 0 {
		t.Fatalf("lapsed lease should be swept, got %d forwarders", n)
	}
}
