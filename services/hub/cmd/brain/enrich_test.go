package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
)

func TestEnrichSnapshotSpawnMeta(t *testing.T) {
	meta := newMetaStore()
	meta.set("s1", spawnMeta{Label: "My Agent", ParentSessionID: "p1", IsSupervisor: true})

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/x","mode":"input"}`), meta)
	var m map[string]any
	_ = json.Unmarshal(out, &m)
	if m["label"] != "My Agent" || m["parentSessionId"] != "p1" || m["isSupervisor"] != true {
		t.Fatalf("spawn metadata not overlaid: %v", m)
	}
	if m["mode"] != "input" {
		t.Error("original fields must be preserved")
	}
}

func TestEnrichSnapshotCwdName(t *testing.T) {
	dir := tempConfigHome(t)
	writeFile(t, filepath.Join(dir, "workspacer", "tui-names.json"), `{"/proj":"Renamed"}`)

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/proj"}`), newMetaStore())
	var m map[string]any
	_ = json.Unmarshal(out, &m)
	if m["label"] != "Renamed" {
		t.Fatalf("cwd rename not applied, got %v", m["label"])
	}
}

func TestEnrichSpawnLabelWinsOverCwdName(t *testing.T) {
	dir := tempConfigHome(t)
	writeFile(t, filepath.Join(dir, "workspacer", "tui-names.json"), `{"/proj":"FromFile"}`)
	meta := newMetaStore()
	meta.set("s1", spawnMeta{Label: "FromSpawn"})

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/proj"}`), meta)
	var m map[string]any
	_ = json.Unmarshal(out, &m)
	if m["label"] != "FromSpawn" {
		t.Fatalf("spawn label should win, got %v", m["label"])
	}
}

func TestSpawnRecordsMeta(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.meta = newMetaStore()

	res, err := reg.handle(context.Background(), "agents.spawn",
		json.RawMessage(`{"cwd":"/tmp","label":"Worker","parentSessionId":"boss"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out struct {
		SessionID string `json:"sessionId"`
	}
	_ = json.Unmarshal(res, &out)
	m, ok := reg.meta.get(out.SessionID)
	if !ok || m.Label != "Worker" || m.ParentSessionID != "boss" {
		t.Fatalf("spawn should record metadata, got %+v (ok=%v)", m, ok)
	}
}

// The store applies enrichment as snapshots land.
func TestStoreEnrichesOnSet(t *testing.T) {
	meta := newMetaStore()
	meta.set("s1", spawnMeta{Label: "Named"})
	s := newSessionStore()
	s.enrich = func(snap json.RawMessage) json.RawMessage { return enrichSnapshot(snap, meta) }

	s.set("s1", json.RawMessage(`{"session_id":"s1"}`))
	snap, _ := s.get("s1")
	var m map[string]any
	_ = json.Unmarshal(snap, &m)
	if m["label"] != "Named" {
		t.Fatalf("store should enrich on set, got %v", m["label"])
	}
}

// The approval card wants the tool's arguments. `pending.raw` is the whole
// PermissionRequest hook payload, so passing it through put the envelope —
// session_id, cwd, hook_event_name — where the command should be. The desktop
// twin has always unwrapped it as `raw.tool_input ?? raw`.
func TestCompatSnapshotUnwrapsTheToolInput(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id": "s1",
		"pending": map[string]any{
			"kind": "approval",
			"tool": "Bash",
			// Exactly what claudemon stores: the hook payload, whole.
			"raw": map[string]any{
				"hook_event_name": "PermissionRequest",
				"session_id":      "s1",
				"cwd":             "/repo",
				"tool_name":       "Bash",
				"tool_input":      map[string]any{"command": "ls -la"},
			},
		},
	})

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	pa, ok := m["pendingApproval"].(map[string]any)
	if !ok {
		t.Fatalf("no pendingApproval in %v", m)
	}
	ti, ok := pa["toolInput"].(map[string]any)
	if !ok {
		t.Fatalf("toolInput is not the tool's arguments: %#v", pa["toolInput"])
	}
	if ti["command"] != "ls -la" {
		t.Errorf("toolInput = %#v, want the command", ti)
	}
	if _, leaked := ti["hook_event_name"]; leaked {
		t.Error("the hook envelope leaked into the approval card")
	}
}

// A payload with no tool_input still shows something rather than nothing.
func TestCompatSnapshotFallsBackToTheWholePayload(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id": "s1",
		"pending": map[string]any{
			"kind": "approval",
			"tool": "Custom",
			"raw":  map[string]any{"detail": "no tool_input here"},
		},
	})
	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	pa := m["pendingApproval"].(map[string]any)
	ti := pa["toolInput"].(map[string]any)
	if ti["detail"] != "no tool_input here" {
		t.Errorf("expected the whole payload as a fallback, got %#v", ti)
	}
}

// sessions.snapshot's fallback (session not in the store) must carry the same
// label/parentSessionId/isSupervisor nesting fields as the main store-backed
// path — that's what enrichAndCompat exists to guarantee for both callers.
// Before the fix, this fallback applied compatSnapshot alone and silently
// dropped them.
func TestSnapshotFallbackEnrichesNestingFields(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/sessions/w1" {
			t.Errorf("unexpected path %q", r.URL.Path)
		}
		_, _ = w.Write([]byte(`{"session_id":"w1","cwd":"/proj","mode":"input"}`))
	}))
	defer srv.Close()

	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.meta = newMetaStore()
	reg.meta.set("w1", spawnMeta{Label: "Worker", ParentSessionID: "boss", IsSupervisor: true})
	// reg.store stays nil — exercising the fallback path.

	res, err := reg.handle(context.Background(), "sessions.snapshot", json.RawMessage(`{"sessionId":"w1"}`))
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal(res, &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if m["label"] != "Worker" || m["parentSessionId"] != "boss" || m["isSupervisor"] != true {
		t.Fatalf("fallback snapshot missing nesting fields: %v", m)
	}
	// The desktop-shape overlay (compatSnapshot) must still be applied too.
	if m["sessionId"] != "w1" || m["sparse"] != true {
		t.Fatalf("fallback snapshot missing desktop-shape overlay: %v", m)
	}
}

func mustJSONBytes(t *testing.T, v any) []byte {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return b
}
