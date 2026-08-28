package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// LAUNCH TRUTH ON THE ROW, not just in the spawn result.
//
// `agents.spawn` answers `fullAccess` — what the session actually runs with —
// and that answer reaches exactly one client: the caller. Every other reader
// (a second tab, /m, the TUI, the same tab after a reload) meets the session as
// a sparse brain row, and a sparse row carried NO permission information at
// all. Each of those clients then fell back to its provider's first mode, so a
// session genuinely running with permissions bypassed displayed as "Ask to
// approve" — the safest possible label on the least safe possible session.
//
// noteLaunch records the same truth onto the spawn metadata and enrichSnapshot
// re-applies it to every row that lands, which is what turns "the caller knows"
// into "everybody knows".

// spawnLaunchTestServer accepts any spawn and echoes an id, so these tests can
// watch what the REGISTRY recorded rather than what claudemon did.
func spawnLaunchTestServer(t *testing.T) *httptest.Server {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			SessionID string `json:"session_id"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		if body.SessionID == "" {
			body.SessionID = "s-managed"
		}
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": body.SessionID})
	}))
	t.Cleanup(srv.Close)
	return srv
}

// spawnAndEnrich runs one agents.spawn and returns both halves of the answer:
// the RESULT the caller got, and the enriched row every other client sees.
// Keeping them together is the point — they must agree.
func spawnAndEnrich(t *testing.T, params string) (result map[string]any, row map[string]any) {
	t.Helper()
	reg := newSpawnTestRegistry(t, spawnLaunchTestServer(t).URL)
	reg.meta = newMetaStore()

	res, err := reg.handle(context.Background(), "agents.spawn", json.RawMessage(params))
	if err != nil {
		t.Fatalf("spawn: %v", err)
	}
	if err := json.Unmarshal(res, &result); err != nil {
		t.Fatalf("unmarshal result: %v", err)
	}
	id, _ := result["sessionId"].(string)
	if id == "" {
		t.Fatalf("spawn returned no sessionId: %v", result)
	}
	enriched := enrichSnapshot(mustJSONBytes(t, map[string]any{
		"session_id": id,
		"cwd":        "/tmp",
		"mode":       "input",
	}), reg.meta)
	if err := json.Unmarshal(enriched, &row); err != nil {
		t.Fatalf("unmarshal row: %v", err)
	}
	return result, row
}

func rowSettings(t *testing.T, row map[string]any) map[string]any {
	t.Helper()
	s, _ := row["settings"].(map[string]any)
	return s
}

// A hub-granted full-access spawn: the row says bypassPermissions, and says the
// launch flag is there (which is also what tells a client's pill that "Full
// access" is a live switch rather than a restart).
func TestLaunchTruthOnTheRowForAGrantedFullAccessSpawn(t *testing.T) {
	result, row := spawnAndEnrich(t,
		`{"cwd":"/tmp","transport":"pty","skipPermissions":true,"yoloGranted":true}`)

	if result["fullAccess"] != true {
		t.Fatalf("precondition: this spawn should have run bypassed, result = %v", result)
	}
	s := rowSettings(t, row)
	if s["permissionMode"] != "bypassPermissions" {
		t.Errorf("row permissionMode = %v, want bypassPermissions", s["permissionMode"])
	}
	if s["bypassAvailable"] != true {
		t.Errorf("row bypassAvailable = %v, want true", s["bypassAvailable"])
	}
	if _, refused := row["escalationScrubbed"]; refused {
		t.Errorf("nothing was refused; the row must not claim otherwise: %v", row["escalationScrubbed"])
	}
	// The original fields survive the overlay.
	if row["mode"] != "input" {
		t.Errorf("enrichment clobbered the row: %v", row)
	}
}

// The refusal case, which is the one that must never round up. An ungranted
// caller asked for full access; the session runs in ask mode, and BOTH the
// result and the row say so — plus what was taken.
func TestLaunchTruthReportsTheClampedModeNotTheRequestedOne(t *testing.T) {
	result, row := spawnAndEnrich(t,
		`{"cwd":"/tmp","transport":"pty","skipPermissions":true,"permissionMode":"bypassPermissions"}`)

	if result["fullAccess"] != false {
		t.Fatalf("precondition: an ungranted spawn must not run bypassed, result = %v", result)
	}
	s := rowSettings(t, row)
	if s["permissionMode"] != "default" {
		t.Errorf("row permissionMode = %v — the REQUEST must not be echoed back as truth", s["permissionMode"])
	}
	if s["bypassAvailable"] != false {
		t.Errorf("row bypassAvailable = %v, want false", s["bypassAvailable"])
	}
	scrubbed, _ := row["escalationScrubbed"].([]any)
	if len(scrubbed) == 0 {
		t.Fatalf("a refused escalation must be visible on the row, got %v", row["escalationScrubbed"])
	}
	// The row's answer is the result's answer — one truth, two readers.
	if !jsonEqual(t, row["escalationScrubbed"], result["escalationScrubbed"]) {
		t.Errorf("row says %v, caller was told %v", row["escalationScrubbed"], result["escalationScrubbed"])
	}
}

// Managed providers have their own two-word vocabulary; the row has to speak it
// or every client's pill renders a mode id it has no label for.
func TestLaunchTruthUsesTheManagedPermissionVocabulary(t *testing.T) {
	_, row := spawnAndEnrich(t, `{"cwd":"/tmp","provider":"codex"}`)
	if got := rowSettings(t, row)["permissionMode"]; got != "ask" {
		t.Errorf("codex row permissionMode = %v, want ask", got)
	}

	_, row = spawnAndEnrich(t, `{"cwd":"/tmp","provider":"codex","skipPermissions":true,"yoloGranted":true}`)
	if got := rowSettings(t, row)["permissionMode"]; got != "yolo" {
		t.Errorf("granted codex row permissionMode = %v, want yolo", got)
	}
}

// Claude on the shipping default transport (stream → spawn-managed) keeps
// Claude's full vocabulary, not the managed pair.
func TestLaunchTruthKeepsClaudeVocabularyOnTheStreamLeg(t *testing.T) {
	_, row := spawnAndEnrich(t, `{"cwd":"/tmp","permissionMode":"plan"}`)
	if got := rowSettings(t, row)["permissionMode"]; got != "plan" {
		t.Errorf("claude-stream row permissionMode = %v, want plan", got)
	}
}

// SILENCE IS THE HONEST ANSWER for a session this brain did not watch start:
// the client says "unknown" and the user can act on that. Filling in a default
// here would recreate the bug one layer down.
func TestUnwatchedSessionCarriesNoPermissionClaim(t *testing.T) {
	var row map[string]any
	enriched := enrichSnapshot(json.RawMessage(`{"session_id":"stranger","cwd":"/tmp"}`), newMetaStore())
	_ = json.Unmarshal(enriched, &row)

	if s := rowSettings(t, row); s != nil {
		if _, ok := s["permissionMode"]; ok {
			t.Errorf("a row we know nothing about must claim no mode, got %v", s["permissionMode"])
		}
		if _, ok := s["bypassAvailable"]; ok {
			t.Errorf("a row we know nothing about must claim no launch flag, got %v", s["bypassAvailable"])
		}
	}
}

// noteLaunch is a MERGE, deliberately: it runs after the wholesale meta.set
// that records label/parent, and clobbering those would break fleet nesting
// for every spawn that named a parent.
func TestNoteLaunchKeepsLabelAndParent(t *testing.T) {
	meta := newMetaStore()
	meta.set("s1", spawnMeta{Label: "Worker", ParentSessionID: "boss", IsWakeTarget: true})
	meta.noteLaunch("s1", "bypassPermissions", true, nil)

	m, ok := meta.get("s1")
	if !ok {
		t.Fatal("metadata vanished")
	}
	if m.Label != "Worker" || m.ParentSessionID != "boss" || !m.IsWakeTarget {
		t.Errorf("noteLaunch clobbered the spawn metadata: %+v", m)
	}
	if !m.LaunchRecorded || m.LaunchPermissionMode != "bypassPermissions" || !m.LaunchFullAccess {
		t.Errorf("launch truth not recorded: %+v", m)
	}
}

// A relaunch onto the same id is a NEW process. The previous life's live
// switches describe one that no longer exists, and leaving them would let a
// stale mode outrank the one this spawn just asked for — the pill would show
// the mode the user restarted AWAY from.
func TestNoteLaunchDropsThePreviousLifesLiveSwitch(t *testing.T) {
	meta := newMetaStore()
	meta.noteLiveControl("s1", "plan", "opus", "high")
	meta.noteLaunch("s1", "bypassPermissions", true, nil)

	m, _ := meta.get("s1")
	if m.LivePermissionMode != "" {
		t.Errorf("a relaunch must not inherit the old life's live mode, got %q", m.LivePermissionMode)
	}
	// The model/effort are the session's own settings, not a permission claim —
	// they carry through as they always have.
	if m.RequestedModel != "opus" || m.LiveEffort != "high" {
		t.Errorf("noteLaunch should touch only the permission fields, got %+v", m)
	}
}

// A live switch AFTER the launch still wins — the launch truth is a floor, not
// a lock. (enrichSnapshot writes livePermissionMode as its own top-level field,
// which every client's pill prefers over settings.permissionMode.)
func TestLiveSwitchAfterLaunchStillShows(t *testing.T) {
	meta := newMetaStore()
	meta.noteLaunch("s1", "bypassPermissions", true, nil)
	meta.noteLiveControl("s1", "plan", "", "")

	var row map[string]any
	_ = json.Unmarshal(enrichSnapshot(json.RawMessage(`{"session_id":"s1"}`), meta), &row)
	if row["livePermissionMode"] != "plan" {
		t.Errorf("live switch lost, row = %v", row)
	}
	if got := rowSettings(t, row)["permissionMode"]; got != "bypassPermissions" {
		t.Errorf("the launch mode should still be recorded underneath, got %v", got)
	}
}

// launchPermissionMode is a TWIN of the desktop's two spawn resolvers
// (claudeSpawn.ts + managedSpawn.ts). A drift here shows up as the same launch
// labelled differently on a desktop row and a headless one.
func TestLaunchPermissionModeMatchesTheDesktopFormula(t *testing.T) {
	cases := []struct {
		provider string
		skip     bool
		asked    string
		want     string
	}{
		{"claude", false, "", "default"},
		{"claude", false, "plan", "plan"},
		{"claude", false, "acceptEdits", "acceptEdits"},
		{"claude", true, "", "bypassPermissions"},
		// skip wins over the asked-for mode: the flag IS full access.
		{"claude", true, "plan", "bypassPermissions"},
		{"codex", false, "", "ask"},
		{"codex", false, "plan", "ask"},
		{"codex", true, "", "yolo"},
		{"opencode", true, "", "yolo"},
		{"pi", false, "", "ask"},
	}
	for _, c := range cases {
		got := launchPermissionMode(c.provider, spawnParams{skip: c.skip, PermissionMode: c.asked})
		if got != c.want {
			t.Errorf("launchPermissionMode(%s, skip=%v, asked=%q) = %q, want %q",
				c.provider, c.skip, c.asked, got, c.want)
		}
	}
}

func jsonEqual(t *testing.T, a, b any) bool {
	t.Helper()
	x, _ := json.Marshal(a)
	y, _ := json.Marshal(b)
	return string(x) == string(y)
}
