package main

// The daemon-owned model facts on the public read surfaces: `requested_selection`
// (what a caller asked for) and `resolved_context_window` (what the daemon's own
// resolver made of it). The brain's job is to PROJECT them onto the camelCase
// names clients read — never to invent one, never to re-pair them, never to
// touch the provider evidence they disagree with.

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// The whole slice is additive: a row from a daemon that has never heard of
// either field must come out the far side without them, so a client cannot tell
// "old daemon" apart from "old daemon" by a null appearing where nothing was.
func TestCompatSnapshotLeavesOldRowsUntouched(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id": "s1",
		"mode":       "input",
		"usage":      map[string]any{"model": "claude-opus-5", "context_tokens": 10},
	})

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	for _, key := range []string{
		"requestedSelection", "requested_selection",
		"resolvedContextWindow", "resolved_context_window",
	} {
		if _, present := m[key]; present {
			t.Errorf("%s appeared on a row that never carried it: %#v", key, m[key])
		}
	}
}

// The full pair, mapped: identity and window travel together under the names
// every client reads, and the snake_case originals stay put for readers that
// deserialize claudemon's own shape (the TUI's Agent).
func TestCompatSnapshotMapsTheOwnerSelection(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id":              "s1",
		"requested_model":         "opus[1m]",
		"requested_selection":     map[string]any{"model": "opus", "context_window": 1_000_000},
		"resolved_context_window": 1_000_000,
	})

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	sel, ok := m["requestedSelection"].(map[string]any)
	if !ok {
		t.Fatalf("requestedSelection missing: %v", m)
	}
	if sel["model"] != "opus" {
		t.Errorf("requestedSelection.model = %v, want opus", sel["model"])
	}
	if got, _ := sel["contextWindow"].(float64); got != 1_000_000 {
		t.Errorf("requestedSelection.contextWindow = %v, want 1000000", sel["contextWindow"])
	}
	if got, _ := m["resolvedContextWindow"].(float64); got != 1_000_000 {
		t.Errorf("resolvedContextWindow = %v, want 1000000", m["resolvedContextWindow"])
	}
	// The compatibility projection older readers know is untouched, and so is
	// the daemon's own spelling.
	if m["requested_model"] != "opus[1m]" {
		t.Errorf("requested_model rewritten: %v", m["requested_model"])
	}
	raw, ok := m["requested_selection"].(map[string]any)
	if !ok || raw["model"] != "opus" {
		t.Errorf("snake_case original dropped: %#v", m["requested_selection"])
	}
}

// A selection with an identity and no window is the daemon saying "opus, window
// not resolved". The projection must preserve that sparseness rather than
// completing the pair from the resolved window sitting right next to it — the
// two answer different questions ("what was asked for" vs "what it came out
// as"), and a client that cannot tell them apart cannot show a mismatch.
func TestCompatSnapshotPreservesASparseSelection(t *testing.T) {
	row := json.RawMessage(`{"session_id":"s1",
	  "requested_selection":{"model":"opus","context_window":null},
	  "resolved_context_window":200000}`)

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	sel, ok := m["requestedSelection"].(map[string]any)
	if !ok {
		t.Fatalf("requestedSelection missing: %v", m)
	}
	if sel["model"] != "opus" {
		t.Errorf("requestedSelection.model = %v", sel["model"])
	}
	window, present := sel["contextWindow"]
	if !present || window != nil {
		t.Errorf("the daemon's unresolved window was rewritten to %#v (present=%v)", window, present)
	}
	if got, _ := m["resolvedContextWindow"].(float64); got != 200_000 {
		t.Errorf("resolvedContextWindow = %v, want 200000", m["resolvedContextWindow"])
	}
}

// Neither field implies the other. A daemon may know what was asked for and not
// what it resolved to (a session that has not spoken yet), or resolve a window
// for a session nobody pinned a selection on.
func TestCompatSnapshotNeverSynthesizesTheMissingHalf(t *testing.T) {
	requestOnly := json.RawMessage(`{"session_id":"s1","requested_selection":{"model":"sonnet","context_window":1000000}}`)
	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(requestOnly), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if _, present := m["resolvedContextWindow"]; present {
		t.Errorf("a request was turned into a resolved window: %#v", m["resolvedContextWindow"])
	}

	resolvedOnly := json.RawMessage(`{"session_id":"s1","resolved_context_window":200000}`)
	m = nil
	if err := json.Unmarshal(compatSnapshot(resolvedOnly), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if _, present := m["requestedSelection"]; present {
		t.Errorf("a resolved window was turned into a request: %#v", m["requestedSelection"])
	}

	// A null resolved window is the daemon's "I don't know", and follows
	// usage.contextLimit's rule: absent rather than a null a client could
	// render as a denominator.
	nulled := json.RawMessage(`{"session_id":"s1","resolved_context_window":null}`)
	m = nil
	if err := json.Unmarshal(compatSnapshot(nulled), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if _, present := m["resolvedContextWindow"]; present {
		t.Errorf("an unknown window arrived as a claim: %#v", m["resolvedContextWindow"])
	}
}

// The early-1M session: 356,380 tokens held against a status line that still
// says the window is 200,000 (Claude Code strips the `[1m]` marker, so its own
// telemetry undercounts until the session outgrows 200k). The daemon resolved
// 1M. The brain must carry BOTH claims through unedited — the resolved window
// under its own name, the provider's contradicted 200,000 exactly as reported —
// because the reconciliation rule (2% drift) lives in the clients, and a client
// can only apply it if it receives both halves.
func TestCompatSnapshotKeepsTheContradictedProviderWindow(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id":              "s1",
		"requested_selection":     map[string]any{"model": "opus", "context_window": 1_000_000},
		"resolved_context_window": 1_000_000,
		"usage": map[string]any{
			"model":          "claude-opus-5",
			"context_tokens": 356_380,
			"context_limit":  1_000_000,
		},
		"status_line": map[string]any{
			"model_display":       "Opus",
			"context_used_pct":    178.19,
			"context_window_size": 200_000,
		},
	})

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got, _ := m["resolvedContextWindow"].(float64); got != 1_000_000 {
		t.Errorf("resolvedContextWindow = %v, want 1000000", m["resolvedContextWindow"])
	}
	u, _ := m["usage"].(map[string]any)
	if got, _ := u["contextLimit"].(float64); got != 1_000_000 {
		t.Errorf("usage.contextLimit = %v, want 1000000", u["contextLimit"])
	}
	sl, _ := m["statusLine"].(map[string]any)
	if got, _ := sl["contextWindowSize"].(float64); got != 200_000 {
		t.Errorf("the provider's own window was rewritten to %v — the evidence must arrive intact", sl["contextWindowSize"])
	}
	if got, _ := sl["contextUsedPct"].(float64); got != 178.19 {
		t.Errorf("contextUsedPct = %v — the contradicted pair travels together or not at all", sl["contextUsedPct"])
	}
	rawSL, _ := m["status_line"].(map[string]any)
	if got, _ := rawSL["context_window_size"].(float64); got != 200_000 {
		t.Errorf("raw status_line.context_window_size = %v", rawSL["context_window_size"])
	}
}

// Every way a client can learn about a session has to say the same thing: the
// boot seed, the live `agent.snapshot` event, and the sessions.snapshot answer
// a reconnecting client asks for. All three go through enrichAndCompat — this
// pins that they still do, byte for byte, for the new fields too.
func TestOwnerSelectionIsIdenticalAcrossSeedEventAndSnapshot(t *testing.T) {
	const rowJSON = `{"session_id":"w1","cwd":"/proj","mode":"input",` +
		`"requested_selection":{"model":"opus","context_window":1000000},` +
		`"resolved_context_window":1000000}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(rowJSON))
	}))
	defer srv.Close()

	meta := newMetaStore()
	store := newSessionStore()
	store.enrich = func(snap json.RawMessage) json.RawMessage { return enrichAndCompat(snap, meta) }

	var published json.RawMessage
	store.onChange = func(_ string, snap json.RawMessage) { published = snap }

	store.seed(map[string]json.RawMessage{"w1": json.RawMessage(rowJSON)})
	seeded, ok := store.get("w1")
	if !ok {
		t.Fatal("seeded row missing")
	}
	store.set("w1", json.RawMessage(rowJSON))
	if published == nil {
		t.Fatal("no snapshot published")
	}

	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.meta = meta
	fallback, err := reg.handle(context.Background(), "sessions.snapshot", json.RawMessage(`{"sessionId":"w1"}`))
	if err != nil {
		t.Fatal(err)
	}

	for name, got := range map[string]json.RawMessage{
		"seed":     seeded,
		"event":    published,
		"snapshot": fallback,
	} {
		var m map[string]any
		if err := json.Unmarshal(got, &m); err != nil {
			t.Fatalf("%s: unmarshal: %v", name, err)
		}
		sel, ok := m["requestedSelection"].(map[string]any)
		if !ok || sel["model"] != "opus" {
			t.Errorf("%s: requestedSelection = %#v", name, m["requestedSelection"])
		}
		if w, _ := sel["contextWindow"].(float64); w != 1_000_000 {
			t.Errorf("%s: requestedSelection.contextWindow = %v", name, sel["contextWindow"])
		}
		if w, _ := m["resolvedContextWindow"].(float64); w != 1_000_000 {
			t.Errorf("%s: resolvedContextWindow = %v", name, m["resolvedContextWindow"])
		}
	}
}
