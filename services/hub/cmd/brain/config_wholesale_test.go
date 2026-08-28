package main

// Fix for the MCP `save_config` bug (2026-08-22): two independent defects.
//
//  1. `projects` (a user-owned map whose entries can be deleted) was wholesale
//     on the TS side only — the brain, which is the SOLE answerer of
//     config.save for every web/mobile/MCP caller, still deep-merged it, so a
//     project delete sent through save_config silently failed to delete.
//  2. The write was based on whatever `current` this call's own read saw, with
//     no check that the file was still THAT when the write actually landed —
//     fine against the other cooperating writer (both take the cross-process
//     lock), but not against a non-participating one (a hand edit, or this
//     process's own unlocked migration writes inside loadFromDisk).
//
// Both are pinned here: contracts/wholesale-config-paths.json for (1), and a
// preWriteHook-driven simulated outside writer for (2) — the same shape as
// briefService.test.ts's "COMPARE-AND-SWAP: an outside writer that ignores
// the lock is not overwritten".

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// TestWholesaleConfigPathsMatchesContract is the cross-language guard: the
// Go brain's wholesaleConfigPaths must name exactly the same dotted paths as
// the TS twin's WHOLESALE_CONFIG_PATHS, both pinned against
// contracts/wholesale-config-paths.json.
func TestWholesaleConfigPathsMatchesContract(t *testing.T) {
	raw := mustReadRepoFile(t, "contracts", "wholesale-config-paths.json")
	var fixture struct {
		Paths []string `json:"paths"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatalf("parse contract fixture: %v", err)
	}
	if len(fixture.Paths) == 0 {
		t.Fatal("contract fixture has no paths — a silently empty fixture guards nothing")
	}
	if !reflect.DeepEqual(wholesaleConfigPaths, fixture.Paths) {
		t.Errorf("wholesaleConfigPaths = %v, fixture says %v", wholesaleConfigPaths, fixture.Paths)
	}
}

// TestConfigSaveReplacesProjectsWholesale proves the brain now agrees with
// configService.ts: `projects` is the whole truth when the caller sends it,
// so deleting a project (resending the full map minus that entry) via
// config.save actually removes it, instead of deep-merge resurrecting it.
func TestConfigSaveReplacesProjectsWholesale(t *testing.T) {
	tempConfigHome(t)

	c := newConfigService()
	c.save(map[string]any{"projects": map[string]any{
		"/home/u/a": map[string]any{"label": "A", "yolo": true},
		"/home/u/b": map[string]any{"label": "B"},
	}})

	// The caller (e.g. the desktop's project settings, or an agent replaying
	// the same wholesale contract via MCP save_config) deletes project b by
	// resending the surviving map.
	merged := c.save(map[string]any{"projects": map[string]any{
		"/home/u/a": map[string]any{"label": "A", "yolo": true},
	}})

	projects := merged["projects"].(map[string]any)
	if _, ok := projects["/home/u/b"]; ok {
		t.Errorf("deleted project b should be gone from merged result, got %v", projects)
	}
	if _, ok := projects["/home/u/a"]; !ok {
		t.Errorf("project a should survive, got %v", projects)
	}
	// fullAccess (yolo) on the surviving project must not have been touched by
	// the delete of its sibling — the exact shape of the 2026-08-20 incident.
	a := projects["/home/u/a"].(map[string]any)
	if a["yolo"] != true {
		t.Errorf("project a's yolo flag was lost by an unrelated delete: %v", a)
	}

	// And it must be gone from disk too.
	fresh := newConfigService().get()
	fp := fresh["projects"].(map[string]any)
	if _, ok := fp["/home/u/b"]; ok {
		t.Errorf("deleted project resurrected after reload from disk: %v", fp)
	}
}

// TestConfigSaveObjectValueRoundTripsAsAnObject proves an object-valued
// setting survives config.save -> config.get unchanged in SHAPE — never
// flattened to a string. Exercised through the same map[string]any path the
// MCP facade's addObjectTool forwards a save_config call through (see
// cmd/mcp/main_test.go's TestFacadeRoutesToolToHub for the facade half of
// this — this is the brain half the facade forwards to).
//
// `pluginSettings` is deliberately a key the shipped defaults have never heard
// of, so this doubles as the guard on the OTHER half of config_orphans.go:
// retirement is a named one-key list, NOT general unknown-key pruning. A key
// absent from the defaults because its feature loads late or lives in a plugin
// must round-trip untouched. (It used to be spelled `supervisor` here, which is
// now on that named list and would be pruned on read.)
func TestConfigSaveObjectValueRoundTripsAsAnObject(t *testing.T) {
	tempConfigHome(t)

	c := newConfigService()
	merged := c.save(map[string]any{
		"pluginSettings": map[string]any{"fullAccess": true, "provider": "claude"},
		"projects": map[string]any{
			"/home/u/proj": map[string]any{"label": "Proj", "yolo": true},
		},
	})

	pluginSettings, ok := merged["pluginSettings"].(map[string]any)
	if !ok {
		t.Fatalf("pluginSettings did not round-trip as an object: %#v", merged["pluginSettings"])
	}
	if pluginSettings["fullAccess"] != true {
		t.Errorf("pluginSettings.fullAccess = %v, want true", pluginSettings["fullAccess"])
	}

	projects, ok := merged["projects"].(map[string]any)
	if !ok {
		t.Fatalf("projects did not round-trip as an object: %#v", merged["projects"])
	}
	proj, ok := projects["/home/u/proj"].(map[string]any)
	if !ok {
		t.Fatalf("projects['/home/u/proj'] did not round-trip as an object: %#v", projects["/home/u/proj"])
	}
	if proj["yolo"] != true {
		t.Errorf("projects['/home/u/proj'].yolo = %v, want true", proj["yolo"])
	}

	// And reading it back fresh (a separate config.get, the way a second bus
	// caller would see it) must show the same shape, not a JSON-encoded string.
	fresh := newConfigService().get()
	if _, ok := fresh["pluginSettings"].(map[string]any); !ok {
		t.Fatalf("pluginSettings on disk is not an object after reload: %#v", fresh["pluginSettings"])
	}
	if _, ok := fresh["projects"].(map[string]any); !ok {
		t.Fatalf("projects on disk is not an object after reload: %#v", fresh["projects"])
	}
}

// TestConfigSaveCASRetriesAgainstAConcurrentWriter is the concurrent-
// modification case: a writer that does NOT participate in the cross-process
// lock (a hand edit, or — inside this very process — the unlocked migration
// writes loadFromDisk can trigger) lands its own change to config.yaml in the
// exact window between this save's read and its write. Without the CAS check,
// the save would recompute nothing and simply overwrite the outsider's change
// with a merge based on the stale read. With it, the save notices the file
// moved and retries against what is actually there — so both changes survive.
//
// Deliberately touches DIFFERENT sections (ours: claude.defaultModel; the
// outsider's: a new project) rather than the same wholesale-replaced key: a
// wholesale key's whole point is that the caller's map IS the truth, so two
// callers racing on the SAME wholesale key is a real conflict with no merge
// to prove — this test is about the ordinary deep-merge path noticing a
// write it did not make, which is the failure the 2026-08-20 incident
// actually describes (an unrelated save clobbering a concurrent one).
func TestConfigSaveCASRetriesAgainstAConcurrentWriter(t *testing.T) {
	dir := tempConfigHome(t)
	p := filepath.Join(dir, "workspacer", "config.yaml")

	c := newConfigService()
	c.save(map[string]any{"projects": map[string]any{
		"/home/u/a": map[string]any{"label": "A"},
	}})

	fired := false
	preWriteHook = func() {
		if fired {
			return // only the first attempt simulates the outsider
		}
		fired = true
		// The outsider writes directly to disk, bypassing withConfigLock
		// entirely (exactly what a hand edit, or an unlocked migration write,
		// would do) — landing a project the read at the top of THIS attempt
		// never saw.
		data, err := os.ReadFile(p)
		if err != nil {
			t.Fatalf("outsider could not read config.yaml: %v", err)
		}
		outside := newConfigService()
		outside.current = outside.loadFromDisk()
		_ = writeConfigYAML(deepMerge(outside.current, map[string]any{
			"projects": map[string]any{"/home/u/outsider": map[string]any{"label": "Outsider"}},
		}))
		if same, _ := os.ReadFile(p); string(same) == string(data) {
			t.Fatal("outsider's write did not change config.yaml — test setup is broken")
		}
	}
	t.Cleanup(func() { preWriteHook = func() {} })

	// Our own save never mentions "projects" at all.
	merged := c.save(map[string]any{"claude": map[string]any{"defaultModel": "opus"}})

	if !fired {
		t.Fatal("preWriteHook never fired — the test did not exercise the CAS path")
	}
	projects := merged["projects"].(map[string]any)
	if _, ok := projects["/home/u/outsider"]; !ok {
		t.Errorf("the outsider's project is missing — the CAS retry did not fold in the concurrent write: %v", projects)
	}
	if _, ok := projects["/home/u/a"]; !ok {
		t.Errorf("the pre-existing project is missing — the retry lost state the outsider didn't touch: %v", projects)
	}
	if merged["claude"].(map[string]any)["defaultModel"] != "opus" {
		t.Errorf("our own partial was lost by the retry: %v", merged["claude"])
	}

	// Confirm on disk, not just the in-memory return value.
	onDisk := newConfigService().get()
	dp := onDisk["projects"].(map[string]any)
	if _, ok := dp["/home/u/outsider"]; !ok {
		t.Errorf("outsider's project did not persist to disk: %v", dp)
	}
	if onDisk["claude"].(map[string]any)["defaultModel"] != "opus" {
		t.Errorf("our own defaultModel did not persist to disk: %v", onDisk["claude"])
	}
}

// TestConfigSaveGivesUpWhenAnOutsiderNeverStops mirrors
// briefService.test.ts's "gives up (writing nothing) when an outside writer
// never stops": if config.yaml changes on EVERY attempt, the CAS can never
// confirm, and save() must refuse rather than spin forever or write over
// whatever is there.
func TestConfigSaveGivesUpWhenAnOutsiderNeverStops(t *testing.T) {
	dir := tempConfigHome(t)
	p := filepath.Join(dir, "workspacer", "config.yaml")

	c := newConfigService()
	n := 0
	preWriteHook = func() {
		outside := newConfigService()
		outside.current = outside.loadFromDisk()
		n++
		_ = writeConfigYAML(deepMerge(outside.current, map[string]any{
			"claude": map[string]any{"defaultModel": "churn"},
		}))
	}
	t.Cleanup(func() { preWriteHook = func() {} })

	before, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}

	got := c.save(map[string]any{"ui": map[string]any{"theme": "nord"}})
	if themeOf(got) == "nord" {
		t.Error("save() reported success while an outsider was churning the file on every attempt")
	}
	if n < saveCASAttempts {
		t.Errorf("preWriteHook fired %d times, want at least %d (one per attempt)", n, saveCASAttempts)
	}
	after, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	if string(before) == string(after) {
		t.Fatal("the outsider's churn never actually landed — test setup is broken")
	}
}
