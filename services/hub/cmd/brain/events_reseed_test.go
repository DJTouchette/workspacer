package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

func TestSessionStoreRecoversLostTerminalOnLagWithoutDisconnect(t *testing.T) {
	var lists atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/sessions":
			w.Header().Set("Content-Type", "application/json")
			if lists.Add(1) == 1 {
				_, _ = w.Write([]byte(`[{"session_id":"worker","mode":"responding"}]`))
			} else {
				if r.URL.Query().Get("include_empty") != "true" {
					t.Error("reconciliation must include unused terminal rows")
				}
				_, _ = w.Write([]byte(`[{"session_id":"worker","mode":"stopped"},{"session_id":"old-history","mode":"stopped"}]`))
			}
		case "/events":
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = w.Write([]byte("event: session.resync\ndata: {\"event\":\"Resync\"}\n\n"))
			w.(http.Flusher).Flush()
			<-r.Context().Done()
		default:
			w.WriteHeader(404)
		}
	}))
	defer server.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	store := newSessionStore()
	changed := make(chan json.RawMessage, 4)
	store.onChange = func(_ string, row json.RawMessage) { changed <- row }
	done := make(chan struct{})
	go func() { defer close(done); runSessionStore(ctx, newClaudemonClient(server.URL), store) }()
	select {
	case row := <-changed:
		if !snapshotEnded(row) {
			t.Fatalf("terminal update not republished: %s", row)
		}
		if _, ok := store.get("old-history"); ok {
			t.Fatal("reconciliation imported unrelated historical sessions")
		}
	case <-time.After(3 * time.Second):
		t.Fatal("lag marker did not reconcile and publish the lost terminal state")
	}
	cancel()
	<-done
}

func TestSessionStoreSeedsAfterEventStreamRecovers(t *testing.T) {
	var attempts atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/sessions" {
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`[{"session_id":"restored","cwd":"/restored","mode":"working"}]`))
			return
		}
		if r.URL.Path == "/events" {
			if attempts.Add(1) == 1 {
				w.WriteHeader(503)
				return
			}
			w.Header().Set("Content-Type", "text/event-stream")
			w.(http.Flusher).Flush()
			// No new event: this session's edge occurred while the brain was disconnected.
			<-r.Context().Done()
			return
		}
		w.WriteHeader(404)
	}))
	defer server.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	store := newSessionStore()
	done := make(chan struct{})
	go func() { defer close(done); runSessionStore(ctx, newClaudemonClient(server.URL), store) }()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		if _, ok := store.get("restored"); ok {
			cancel()
			<-done
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("session existing before SSE recovery was never seeded")
}
