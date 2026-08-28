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

// The web/mobile UI reads ONLY the camelCase `statusLine` overlay
// (sessionStats.ts's deriveSessionStats), never the raw `status_line`. Before
// this fix, a high-frequency statusline tick (the only thing that arrives
// during a turn with no tool calls, since that is what would otherwise trigger
// claudemon's /events session.update and a full re-enrich) refreshed
// `status_line` but left `statusLine` stale or entirely missing — so the model
// name and every other camelCase-only reader went blank for the whole running
// turn and only caught up once the turn ended.
func TestUpdateStatusLineRefreshesTheCamelOverlay(t *testing.T) {
	s := newSessionStore()
	// Seeded with no status line at all yet, mirroring a freshly spawned
	// session's first snapshot.
	s.seed(map[string]json.RawMessage{"s1": json.RawMessage(`{"session_id":"s1","mode":"responding"}`)})

	snap, _ := s.get("s1")
	var before map[string]any
	_ = json.Unmarshal(snap, &before)
	if _, present := before["statusLine"]; present {
		t.Fatalf("test setup: expected no statusLine overlay before any tick, got %v", before["statusLine"])
	}

	s.updateStatusLine("s1", json.RawMessage(`{"model_display":"Opus 4.8","cost_usd":1.5,"context_used_pct":12.5}`))

	snap, _ = s.get("s1")
	var after map[string]any
	_ = json.Unmarshal(snap, &after)
	sl, ok := after["statusLine"].(map[string]any)
	if !ok {
		t.Fatalf("statusLine overlay was not created by updateStatusLine: %v", after)
	}
	if sl["modelDisplay"] != "Opus 4.8" {
		t.Errorf("statusLine.modelDisplay = %v, want %q", sl["modelDisplay"], "Opus 4.8")
	}
	if sl["costUSD"] != 1.5 {
		t.Errorf("statusLine.costUSD = %v, want 1.5", sl["costUSD"])
	}
	if sl["contextUsedPct"] != 12.5 {
		t.Errorf("statusLine.contextUsedPct = %v, want 12.5", sl["contextUsedPct"])
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
