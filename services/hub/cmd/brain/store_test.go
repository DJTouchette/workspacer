package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestSessionStoreSetGetAll(t *testing.T) {
	s := newSessionStore()
	fired := make(chan string, 4)
	s.onChange = func(id string, _ json.RawMessage) { fired <- id }

	s.seed(map[string]json.RawMessage{"a": json.RawMessage(`{"session_id":"a"}`)})
	if len(fired) != 0 {
		t.Fatal("seed must not fire onChange")
	}
	s.set("b", json.RawMessage(`{"session_id":"b"}`))
	if got := <-fired; got != "b" {
		t.Fatalf("onChange id = %q, want b", got)
	}
	if _, ok := s.get("a"); !ok {
		t.Fatal("seeded session a missing")
	}
	all := s.all()
	if len(all) != 2 || snapshotID(all[0]) != "a" || snapshotID(all[1]) != "b" {
		t.Fatalf("all() should be [a b] sorted, got %v", all)
	}
}

// TestLiveSessionStore drives runSessionStore against a fake claudemon: it seeds
// from /sessions, follows /events, and on a session.update frame refreshes the
// session via /sessions/:id and publishes the change.
func TestLiveSessionStore(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("/sessions", func(w http.ResponseWriter, _ *http.Request) {
		w.Write([]byte(`[{"session_id":"s1","mode":"seed"}]`))
	})
	mux.HandleFunc("/sessions/s1", func(w http.ResponseWriter, _ *http.Request) {
		w.Write([]byte(`{"session_id":"s1","mode":"updated"}`)) // the canonical refresh
	})
	mux.HandleFunc("/events", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		io.WriteString(w, "event: session.update\ndata: {\"session_id\":\"s1\"}\n\n")
		if fl, ok := w.(http.Flusher); ok {
			fl.Flush()
		}
		<-r.Context().Done() // hold the stream open like real SSE
	})
	srv := httptest.NewServer(mux)
	defer srv.Close()

	store := newSessionStore()
	published := make(chan json.RawMessage, 4)
	store.onChange = func(_ string, snap json.RawMessage) { published <- snap }

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go runSessionStore(ctx, newClaudemonClient(srv.URL), store)

	select {
	case snap := <-published:
		var m map[string]any
		_ = json.Unmarshal(snap, &m)
		if m["mode"] != "updated" {
			t.Fatalf("published snapshot should be the refreshed one, got %v", m["mode"])
		}
	case <-time.After(3 * time.Second):
		t.Fatal("no agent.snapshot published within 3s")
	}

	snap, ok := store.get("s1")
	if !ok {
		t.Fatal("store missing s1 after update")
	}
	var m map[string]any
	_ = json.Unmarshal(snap, &m)
	if m["mode"] != "updated" {
		t.Errorf("stored snapshot mode = %v, want updated", m["mode"])
	}
	if len(store.all()) != 1 {
		t.Errorf("store should hold exactly one session, got %d", len(store.all()))
	}
}
