package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

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
