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

func TestUpdateStatusLineMergesKnownSkipsUnknown(t *testing.T) {
	s := newSessionStore()
	s.seed(map[string]json.RawMessage{"s1": json.RawMessage(`{"session_id":"s1","mode":"input"}`)})

	s.updateStatusLine("s1", json.RawMessage(`{"cost_usd":1.5}`))
	snap, _ := s.get("s1")
	var m map[string]any
	_ = json.Unmarshal(snap, &m)
	sl, _ := m["status_line"].(map[string]any)
	if sl["cost_usd"] != 1.5 || m["mode"] != "input" {
		t.Fatalf("status_line should merge, other fields preserved: %v", m)
	}

	// Unknown session: no-op, no panic.
	s.updateStatusLine("ghost", json.RawMessage(`{"cost_usd":9}`))
	if _, ok := s.get("ghost"); ok {
		t.Error("unknown session must not be created")
	}
}

func TestRunStatusLines(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("/statusline/stream", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		io.WriteString(w, "event: statusline\ndata: {\"session_id\":\"s1\",\"status_line\":{\"cost_usd\":2.5}}\n\n")
		if fl, ok := w.(http.Flusher); ok {
			fl.Flush()
		}
		<-r.Context().Done()
	})
	srv := httptest.NewServer(mux)
	defer srv.Close()

	store := newSessionStore()
	store.seed(map[string]json.RawMessage{"s1": json.RawMessage(`{"session_id":"s1"}`)})

	pushed := make(chan json.RawMessage, 4)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go runStatusLines(ctx, newClaudemonClient(srv.URL), store, func(_ string, sl json.RawMessage) { pushed <- sl })

	select {
	case sl := <-pushed:
		var m map[string]any
		_ = json.Unmarshal(sl, &m)
		if m["cost_usd"] != 2.5 {
			t.Fatalf("pushed statusline wrong: %v", m)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("no statusline pushed within 3s")
	}

	snap, _ := store.get("s1")
	var m map[string]any
	_ = json.Unmarshal(snap, &m)
	if sl, _ := m["status_line"].(map[string]any); sl["cost_usd"] != 2.5 {
		t.Errorf("store snapshot should carry the live status_line, got %v", m["status_line"])
	}
}
