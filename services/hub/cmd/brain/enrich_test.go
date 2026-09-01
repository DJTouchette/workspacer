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
	meta.set("s1", spawnMeta{Label: "My Agent", ParentSessionID: "p1", IsWakeTarget: true})

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/x","mode":"input"}`), meta)
	var m map[string]any
	_ = json.Unmarshal(out, &m)
	if m["label"] != "My Agent" || m["parentSessionId"] != "p1" || m["isWakeTarget"] != true {
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
// label/parentSessionId/isWakeTarget nesting fields as the main store-backed
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
	reg.meta.set("w1", spawnMeta{Label: "Worker", ParentSessionID: "boss", IsWakeTarget: true})
	// reg.store stays nil — exercising the fallback path.

	res, err := reg.handle(context.Background(), "sessions.snapshot", json.RawMessage(`{"sessionId":"w1"}`))
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal(res, &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if m["label"] != "Worker" || m["parentSessionId"] != "boss" || m["isWakeTarget"] != true {
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

// The prompt-cache split has to survive the camelCase projection, or the
// desktop shows it and every hub client (mobile /m, web /app, wks-tui) does
// not. That is the exact silent starvation this projection produced before with
// statusLine. It is passed through whole because claudemon already names its
// sub-keys the way the desktop reads them.
func TestCompatSnapshotCarriesTheCacheSplit(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id": "s1",
		"usage": map[string]any{
			"model":          "claude-opus-5",
			"context_tokens": 40984,
			"context_limit":  200000,
			"cost_usd":       1.5,
			"cache":          map[string]any{"fresh": 2, "write": 23393, "read": 17589},
		},
	})

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	u, ok := m["usage"].(map[string]any)
	if !ok {
		t.Fatalf("no usage in %v", m)
	}
	c, ok := u["cache"].(map[string]any)
	if !ok {
		t.Fatalf("cache split dropped from the projection: %#v", u)
	}
	for key, want := range map[string]float64{"fresh": 2, "write": 23393, "read": 17589} {
		if got, _ := c[key].(float64); got != want {
			t.Errorf("cache.%s = %v, want %v", key, c[key], want)
		}
	}
	// The camelCase counters still land alongside it.
	if got, _ := u["contextTokens"].(float64); got != 40984 {
		t.Errorf("contextTokens = %v", u["contextTokens"])
	}
}

// A session whose provider reported no cache data must not arrive carrying a
// null `cache` key: "did not say" and "cached nothing" are different claims,
// and a present-but-null key invites a client to render the second.
func TestCompatSnapshotOmitsAnUnreportedCacheSplit(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id": "s1",
		"usage":      map[string]any{"model": "gpt-5-codex", "context_tokens": 10},
	})

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	u, _ := m["usage"].(map[string]any)
	if _, present := u["cache"]; present {
		t.Errorf("usage.cache present for a session that reported none: %#v", u)
	}
}

// The Codex half of the same journey: the cache-read subset claudemon already
// used to discount the cost estimate now has to reach a client too.
func TestCompatSnapshotCarriesCachedInputTokens(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id": "s1",
		"status_line": map[string]any{
			"model_display":       "gpt-5-codex",
			"total_input_tokens":  4402946,
			"cached_input_tokens": 3733376,
			"context_usage_state": "waiting_for_runtime_usage",
		},
	})

	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	sl, ok := m["statusLine"].(map[string]any)
	if !ok {
		t.Fatalf("no statusLine in %v", m)
	}
	if got, _ := sl["cachedInputTokens"].(float64); got != 3733376 {
		t.Errorf("cachedInputTokens = %v, want 3733376", sl["cachedInputTokens"])
	}
	if got := sl["contextUsageState"]; got != "waitingForRuntimeUsage" {
		t.Errorf("contextUsageState = %v, want waitingForRuntimeUsage", got)
	}
}

// ── unknown is not idle, and background work is not idle ─────────────────────

// A session that is SPAWNING sits in claudemon's `unknown` mode: no hook and no
// driver event has arrived yet. So does a resume the moment register_spawn
// flips a Stopped row back, and so does a terminal PTY for its whole life.
//
// `ambientForMode` used to answer all three with `default: return "idle"`, and
// the headless path is the ONLY thing that answers the question for /m, the web
// renderer, and anything else reading the bus with no desktop present. A
// starting agent therefore reported "finished" on the wire.
//
// The desktop's twin (claudeSessionStore.applyManagedMode) returns early on
// unknown and leaves the previous state alone. A stateless overlay's equivalent
// is to emit NO ambientState, which the sparse-merge on the other end reads as
// "nothing new to say" rather than as a claim.
func TestCompatSnapshotDoesNotCallASpawningSessionIdle(t *testing.T) {
	for _, mode := range []string{"unknown", "", "some_mode_from_a_newer_daemon"} {
		row := mustJSONBytes(t, map[string]any{"session_id": "s1", "mode": mode})
		var m map[string]any
		if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
			t.Fatalf("unmarshal: %v", err)
		}
		if got, present := m["ambientState"]; present {
			t.Errorf("mode %q emitted ambientState %q — a session nobody has heard from is not in any ambient state, and calling it idle tells every headless client a starting agent has finished", mode, got)
		}
		// The row must still be a live one: only `stopped` ends a session.
		if m["status"] != "active" {
			t.Errorf("mode %q gave status %v, want active", mode, m["status"])
		}
	}
}

// The four modes the desktop vocabulary CAN express still map, so omitting the
// unknown ones did not quietly cost the overlay its actual job.
func TestCompatSnapshotMapsTheKnownModes(t *testing.T) {
	for mode, want := range map[string]string{
		"responding": "streaming",
		"approval":   "waiting_approval",
		"question":   "waiting_input",
		"input":      "idle",
		"stopped":    "idle",
	} {
		row := mustJSONBytes(t, map[string]any{"session_id": "s1", "mode": mode})
		var m map[string]any
		if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
			t.Fatalf("unmarshal: %v", err)
		}
		if m["ambientState"] != want {
			t.Errorf("mode %q → ambientState %v, want %q", mode, m["ambientState"], want)
		}
	}
}

// `run_in_background` shells (a dev server, a watcher, an agent-authored poll
// loop) deliberately do NOT hold the session mode busy — claudemon latched
// sessions "responding" forever when they did. They ride the wire as
// `background_tasks` instead, and the overlay consulted it nowhere: the count
// never reached clients under the name they read, and mode `input` produced a
// flat "idle" while a build was running.
func TestCompatSnapshotSurfacesBackgroundWork(t *testing.T) {
	row := mustJSONBytes(t, map[string]any{
		"session_id": "s1", "mode": "input", "background_tasks": 2,
	})
	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if m["ambientState"] != "background" {
		t.Errorf("ambientState = %v, want background — the agent's own turn ended but the work it started is still running", m["ambientState"])
	}
	if got := m["backgroundTasks"]; got != float64(2) {
		t.Errorf("backgroundTasks = %v (%T), want 2 — the count clients read is camelCase", got, got)
	}
}

// Zero background tasks must not manufacture a claim: no count, and idle stays
// idle. An absent `backgroundTasks` is what every pre-field row means.
func TestCompatSnapshotOmitsAnEmptyBackgroundCount(t *testing.T) {
	for _, row := range []json.RawMessage{
		mustJSONBytes(t, map[string]any{"session_id": "s1", "mode": "input"}),
		mustJSONBytes(t, map[string]any{"session_id": "s1", "mode": "input", "background_tasks": 0}),
	} {
		var m map[string]any
		if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
			t.Fatalf("unmarshal: %v", err)
		}
		if _, present := m["backgroundTasks"]; present {
			t.Errorf("emitted backgroundTasks for a row with none: %v", m["backgroundTasks"])
		}
		if m["ambientState"] != "idle" {
			t.Errorf("ambientState = %v, want idle", m["ambientState"])
		}
	}
}

// Background work does not outrank being blocked on a human, or being busy:
// the fold applies to mode `input` only.
func TestCompatSnapshotBackgroundDoesNotMaskTheMode(t *testing.T) {
	for mode, want := range map[string]string{
		"responding": "streaming",
		"approval":   "waiting_approval",
		"question":   "waiting_input",
	} {
		row := mustJSONBytes(t, map[string]any{
			"session_id": "s1", "mode": mode, "background_tasks": 3,
		})
		var m map[string]any
		if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
			t.Fatalf("unmarshal: %v", err)
		}
		if m["ambientState"] != want {
			t.Errorf("mode %q with background work → %v, want %q", mode, m["ambientState"], want)
		}
	}
}
