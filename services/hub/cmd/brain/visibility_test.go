package main

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"
	"time"
)

// ── snapshotVisible: the shared desktop fleet-visibility rule ────────────────

func TestSnapshotVisible(t *testing.T) {
	now := time.Date(2026, 7, 10, 12, 0, 0, 0, time.UTC)
	recent := now.Add(-time.Hour).Format(time.RFC3339)
	old := now.Add(-48 * time.Hour).Format(time.RFC3339)
	row := func(id, mode, updatedAt string, archived bool) json.RawMessage {
		return json.RawMessage(fmt.Sprintf(
			`{"session_id":%q,"mode":%q,"updated_at":%q,"archived":%v}`, id, mode, updatedAt, archived))
	}
	curated := map[string]bool{"cur": true}

	cases := []struct {
		name      string
		snap      json.RawMessage
		layoutIDs map[string]bool
		hasLayout bool
		want      bool
	}{
		// Live sessions — idle included — are always visible.
		{"live idle", row("a", "input", recent, false), nil, false, true},
		{"live responding", row("a", "responding", recent, false), curated, true, true},
		{"live waiting approval", row("a", "approval", old, false), curated, true, true},
		// mode unknown = no hook/managed signal yet (shell terminals, TUI
		// startup) — the desktop store never contains these.
		{"unknown (terminal)", row("a", "unknown", recent, false), curated, true, false},
		{"unknown no layout", row("a", "unknown", recent, false), nil, false, false},
		// Stopped: only when curated by the layout…
		{"stopped curated", row("cur", "stopped", old, false), curated, true, true},
		{"stopped uncurated", row("a", "stopped", recent, false), curated, true, false},
		// …or recently stopped when no layout exists at all.
		{"stopped recent no layout", row("a", "stopped", recent, false), nil, false, true},
		{"stopped old no layout", row("a", "stopped", old, false), nil, false, false},
		{"stopped recent archived no layout", row("a", "stopped", recent, true), nil, false, false},
		{"stopped no timestamp no layout", row("a", "stopped", "", false), nil, false, false},
		// A desktop-shaped ended row (status, no mode) counts as stopped.
		{"desktop-shaped ended", json.RawMessage(`{"sessionId":"a","status":"ended"}`), nil, true, false},
		{"malformed", json.RawMessage(`{`), nil, false, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := snapshotVisible(tc.snap, tc.layoutIDs, tc.hasLayout, now); got != tc.want {
				t.Errorf("snapshotVisible() = %v, want %v", got, tc.want)
			}
		})
	}
}

// ── layoutSessionIDs: curated ids from the shared layout document ────────────

func TestLayoutSessionIDs(t *testing.T) {
	doc := json.RawMessage(`{"version":7,"data":{"agents":[
		{"id":"global","global":true,"lastSessionId":"global-last","tabs":[]},
		{"id":"a1","sessionId":"live-1","tabs":[{"panes":[{"attachSessionId":"attached-1"}]}]},
		{"id":"a2","lastSessionId":"stopped-2","tabs":[]}
	]}}`)
	ids, hasLayout := layoutSessionIDs(doc)
	if !hasLayout {
		t.Fatal("a populated document must report hasLayout")
	}
	for _, want := range []string{"live-1", "attached-1", "stopped-2"} {
		if !ids[want] {
			t.Errorf("missing curated id %q in %v", want, ids)
		}
	}
	// The global Overview workspace is not an agent card — its ids don't curate.
	if ids["global-last"] {
		t.Error("global workspace ids must not curate stopped sessions")
	}

	// Null / absent documents mean "no layout" → the recency fallback applies.
	for _, raw := range []string{`{"version":0,"data":null}`, `null`, ``} {
		if _, has := layoutSessionIDs(json.RawMessage(raw)); has {
			t.Errorf("layoutSessionIDs(%q) reported a layout", raw)
		}
	}
	// An empty-but-present agents array IS a layout (everything closed on purpose).
	if _, has := layoutSessionIDs(json.RawMessage(`{"version":1,"data":{"agents":[]}}`)); !has {
		t.Error("an empty agents array should still count as a layout")
	}
}

// ── visibility: layout fetch caching + failure fallback ──────────────────────

func TestVisibilityCachesLayoutFetch(t *testing.T) {
	fetches := 0
	v := newVisibility(func(context.Context) (json.RawMessage, error) {
		fetches++
		return json.RawMessage(`{"version":1,"data":{"agents":[{"id":"a","sessionId":"s1"}]}}`), nil
	}, time.Hour)
	ctx := context.Background()

	stopped := json.RawMessage(`{"session_id":"s1","mode":"stopped"}`)
	if !v.visible(ctx, stopped) {
		t.Fatal("curated stopped session should be visible")
	}
	if v.visible(ctx, json.RawMessage(`{"session_id":"other","mode":"stopped","updated_at":"2020-01-01T00:00:00Z"}`)) {
		t.Fatal("uncurated stopped session should be hidden when a layout exists")
	}
	if fetches != 1 {
		t.Fatalf("layout fetched %d times within TTL, want 1", fetches)
	}
}

func TestVisibilityKeepsLastLayoutOnFetchError(t *testing.T) {
	fail := false
	v := newVisibility(func(context.Context) (json.RawMessage, error) {
		if fail {
			return nil, fmt.Errorf("hub away")
		}
		return json.RawMessage(`{"version":1,"data":{"agents":[{"id":"a","sessionId":"s1"}]}}`), nil
	}, time.Nanosecond) // effectively no cache: every check re-fetches
	ctx := context.Background()

	curated := json.RawMessage(`{"session_id":"s1","mode":"stopped"}`)
	if !v.visible(ctx, curated) {
		t.Fatal("curated stopped session should be visible")
	}
	fail = true
	time.Sleep(time.Millisecond)
	if !v.visible(ctx, curated) {
		t.Fatal("a fetch failure must fall back to the last known layout")
	}
}

// ── compatSnapshot: desktop-shape overlay on claudemon rows ──────────────────

func TestCompatSnapshotMapsDesktopFields(t *testing.T) {
	raw := json.RawMessage(`{
		"session_id":"s1","mode":"responding","cwd":"/tmp/p","provider":"claude",
		"transport":"stream","archived":false,"updated_at":"2026-07-10T12:00:00Z",
		"usage":{"model":"m1","context_tokens":100,"context_limit":200000,"cost_usd":1.5},
		"pending":null}`)
	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(raw), &m); err != nil {
		t.Fatal(err)
	}
	if m["sessionId"] != "s1" || m["status"] != "active" || m["ambientState"] != "streaming" {
		t.Fatalf("core overlay wrong: %v %v %v", m["sessionId"], m["status"], m["ambientState"])
	}
	if m["sparse"] != true {
		t.Error("brain rows must be marked sparse so rich desktop snapshots aren't clobbered")
	}
	if m["session_id"] != "s1" || m["mode"] != "responding" {
		t.Error("snake_case originals must be preserved")
	}
	want := time.Date(2026, 7, 10, 12, 0, 0, 0, time.UTC).UnixMilli()
	if int64(m["lastActivity"].(float64)) != want {
		t.Errorf("lastActivity = %v, want %d", m["lastActivity"], want)
	}
	u := m["usage"].(map[string]any)
	if u["contextTokens"] != float64(100) || u["contextLimit"] != float64(200000) ||
		u["costUSD"] != 1.5 || u["model"] != "m1" {
		t.Errorf("usage not camelCased: %v", u)
	}
	// Both pending fields must be present (null) so a client merge clears them.
	for _, k := range []string{"pendingApproval", "pendingQuestions"} {
		if v, ok := m[k]; !ok || v != nil {
			t.Errorf("%s should be explicitly null, got %v (present %v)", k, v, ok)
		}
	}
}

func TestCompatSnapshotStatusAndAmbient(t *testing.T) {
	cases := []struct{ mode, status, ambient string }{
		{"stopped", "ended", "idle"},
		{"input", "active", "idle"},
		{"unknown", "active", "idle"},
		{"responding", "active", "streaming"},
		{"approval", "active", "waiting_approval"},
		{"question", "active", "waiting_input"},
	}
	for _, tc := range cases {
		raw := json.RawMessage(fmt.Sprintf(`{"session_id":"s","mode":%q}`, tc.mode))
		var m map[string]any
		_ = json.Unmarshal(compatSnapshot(raw), &m)
		if m["status"] != tc.status || m["ambientState"] != tc.ambient {
			t.Errorf("mode %s → status %v ambient %v, want %s/%s",
				tc.mode, m["status"], m["ambientState"], tc.status, tc.ambient)
		}
	}
}

func TestCompatSnapshotPending(t *testing.T) {
	approval := json.RawMessage(`{"session_id":"s","mode":"approval",
		"pending":{"kind":"approval","tool":"Bash","summary":"rm -rf","raw":{"command":"rm -rf /tmp/x"}}}`)
	var m map[string]any
	_ = json.Unmarshal(compatSnapshot(approval), &m)
	pa, _ := m["pendingApproval"].(map[string]any)
	if pa == nil || pa["toolName"] != "Bash" {
		t.Fatalf("pendingApproval = %v, want toolName Bash", m["pendingApproval"])
	}
	if ti, _ := pa["toolInput"].(map[string]any); ti["command"] != "rm -rf /tmp/x" {
		t.Errorf("toolInput should carry the raw payload, got %v", pa["toolInput"])
	}

	question := json.RawMessage(`{"session_id":"s","mode":"question",
		"pending":{"kind":"question","questions":[{"question":"Which?","options":[{"label":"A"},{"label":"B","description":"b"}]}]}}`)
	m = map[string]any{}
	_ = json.Unmarshal(compatSnapshot(question), &m)
	qs, _ := m["pendingQuestions"].([]any)
	if len(qs) != 1 {
		t.Fatalf("pendingQuestions = %v, want the claudemon questions array", m["pendingQuestions"])
	}
	if q := qs[0].(map[string]any); q["question"] != "Which?" {
		t.Errorf("question payload lost: %v", q)
	}
}

func TestCompatSnapshotLeavesNonClaudemonRows(t *testing.T) {
	desktop := json.RawMessage(`{"sessionId":"s1","status":"active","conversation":[]}`)
	if got := compatSnapshot(desktop); string(got) != string(desktop) {
		t.Errorf("a row without session_id must pass through untouched, got %s", got)
	}
}

// ── registry: agents.list / sessions.snapshots serve the visible set ─────────

func TestListAndSnapshotsApplyVisibilityRule(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	store := newSessionStore()
	store.enrich = compatSnapshot // same overlay main.go wires in
	now := time.Now().UTC()
	seed := map[string]json.RawMessage{}
	for id, row := range map[string]string{
		"live-idle":       `{"session_id":"live-idle","mode":"input","updated_at":"` + now.Format(time.RFC3339) + `"}`,
		"live-working":    `{"session_id":"live-working","mode":"responding"}`,
		"terminal":        `{"session_id":"terminal","mode":"unknown"}`,
		"stopped-curated": `{"session_id":"stopped-curated","mode":"stopped","updated_at":"2020-01-01T00:00:00Z"}`,
		"stopped-old":     `{"session_id":"stopped-old","mode":"stopped","updated_at":"2020-01-01T00:00:00Z"}`,
	} {
		seed[id] = json.RawMessage(row)
	}
	store.seed(seed)
	reg.store = store
	reg.vis = newVisibility(func(context.Context) (json.RawMessage, error) {
		return json.RawMessage(`{"version":1,"data":{"agents":[{"id":"a","lastSessionId":"stopped-curated"}]}}`), nil
	}, time.Hour)

	for _, method := range []string{"agents.list", "sessions.snapshots"} {
		res, err := reg.handle(context.Background(), method, nil)
		if err != nil {
			t.Fatalf("%s: %v", method, err)
		}
		var rows []map[string]any
		if err := json.Unmarshal(res, &rows); err != nil {
			t.Fatalf("%s: %v", method, err)
		}
		got := map[string]bool{}
		for _, r := range rows {
			id, _ := r["sessionId"].(string)
			if id == "" {
				t.Errorf("%s: row without camelCase sessionId: %v", method, r)
			}
			got[id] = true
		}
		want := []string{"live-idle", "live-working", "stopped-curated"}
		if len(got) != len(want) {
			t.Errorf("%s returned %v, want exactly %v", method, got, want)
		}
		for _, id := range want {
			if !got[id] {
				t.Errorf("%s: missing %s", method, id)
			}
		}
	}
}

// ── busClient: outbound call reply routing ────────────────────────────────────

func TestBusClientDispatchRoutesCallReplies(t *testing.T) {
	b := newBusClient("ws://unused", "", nil, nil)
	ch := make(chan frame, 1)
	b.callMu.Lock()
	b.calls["brain-1"] = ch
	b.callMu.Unlock()

	b.dispatch(context.Background(), frame{Op: "result", ID: "brain-1", Result: json.RawMessage(`{"ok":true}`)})
	select {
	case f := <-ch:
		if string(f.Result) != `{"ok":true}` {
			t.Fatalf("routed result = %s", f.Result)
		}
	default:
		t.Fatal("result frame not routed to the pending call")
	}
	// Unknown ids (or late replies after timeout cleanup) are dropped quietly.
	b.dispatch(context.Background(), frame{Op: "error", ID: "gone", Error: "boom"})
}
