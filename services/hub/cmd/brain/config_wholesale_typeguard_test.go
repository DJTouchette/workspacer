package main

// THE CONFIG WIPE (found 2026-08-29, release-readiness sweep).
//
// applyWholesale ended:
//
//	if vMap, ok := v.(map[string]any); ok { dst[leaf] = vMap }
//	else                                  { dst[leaf] = map[string]any{} }
//
// A wholesale path is REPLACED rather than merged, so that `else` did not
// degrade a bad value — it DELETED the user's whole map (every project's label,
// colour, icon, favourite flag, delivery mode, yolo flag and worktreeSetup
// hooks; every custom theme; every budget) and returned the emptied config as a
// SUCCESSFUL save. Nothing is backed up on a successful write, so there was
// nothing to restore from either.
//
// It was on the hot path, not in a corner: an agent's config writes ALWAYS
// answer here (contracts/wholesale-config-paths.json), and the MCP facade
// registered save_config with no input schema, so a client that serialised its
// argument sent `projects` as a string and was coerced rather than refused.
//
// Three layers now: the door (cmd/mcp configsave_test.go), the writer (here and
// the TS twin), and the fixture that holds the two writers together
// (TestWholesaleValueContractCases). These are the WRITER's tests, end to end
// through save() and the file on disk — the fixture exercises the pure function.

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// seedTwoProjects writes a config with two projects AND one ordinary
// non-default setting, then returns the config service.
//
// The extra setting is load-bearing scaffolding, and it is worth saying why: on
// a config that is otherwise pristine, the wipe is INVISIBLE, because the merged
// document comes out exactly equal to the shipped defaults and
// refuseWipeWithDefaults declines the write for an unrelated reason. So the
// first draft of this test passed against the shipped bug. A real user — anyone
// who has ever changed a theme — is not in that state, and neither is this
// fixture.
func seedTwoProjects(t *testing.T) *configService {
	t.Helper()
	c := newConfigService()
	mustSave(t, c, map[string]any{"ui": map[string]any{"theme": "nord"}})
	mustSave(t, c, map[string]any{"projects": map[string]any{
		"/home/u/a": map[string]any{"label": "A", "yolo": true},
		"/home/u/b": map[string]any{"label": "B"},
	}})
	return c
}

func projectsOnDisk(t *testing.T) map[string]any {
	t.Helper()
	fresh := newConfigService().get()
	projects, ok := fresh["projects"].(map[string]any)
	if !ok {
		t.Fatalf("projects is no longer a map on disk: %#v", fresh["projects"])
	}
	return projects
}

// TestConfigSaveRefusesANonMapAtAWholesalePath is the defect itself: a save
// whose `projects` arrives as a STRING must be refused with nothing written,
// rather than answered by emptying the map and reporting success.
func TestConfigSaveRefusesANonMapAtAWholesalePath(t *testing.T) {
	tempConfigHome(t)
	c := seedTwoProjects(t)

	before, err := os.ReadFile(filepath.Join(configDir(), "config.yaml"))
	if err != nil {
		t.Fatal(err)
	}

	// The trigger: a client that serialised its argument.
	got, saveErr := c.save(map[string]any{"projects": `{"/home/u/a":{"label":"A"}}`})

	if saveErr == nil {
		t.Fatal("save() reported SUCCESS for a malformed wholesale value — this is the defect: it used to answer by deleting the map")
	}
	if !errors.Is(saveErr, errWholesaleNotAMap) {
		t.Errorf("save() refused with %v, want errWholesaleNotAMap", saveErr)
	}
	// The refusal must name the path and say nothing was written; a user reading
	// this message has to know which key to resend.
	if !strings.Contains(saveErr.Error(), "projects") {
		t.Errorf("the refusal does not name the offending path: %v", saveErr)
	}

	// Nothing written, and the returned value still describes the file.
	after, err := os.ReadFile(filepath.Join(configDir(), "config.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	if string(before) != string(after) {
		t.Errorf("a REFUSED save still rewrote config.yaml:\nbefore:\n%s\nafter:\n%s", before, after)
	}
	if p, ok := got["projects"].(map[string]any); !ok || len(p) != 2 {
		t.Errorf("the value handed back to the caller is not the surviving config: %#v", got["projects"])
	}
	if p := projectsOnDisk(t); len(p) != 2 {
		t.Fatalf("a malformed save DELETED the user's projects: %#v", p)
	}
}

// TestConfigSaveRefusalIsAllOrNothing: the merge has already folded the
// caller's OTHER keys in by the time the wholesale value is judged, so a save
// that refuses one path must refuse the whole call. Applying the rest is how a
// caller learns its save "worked" while one map silently did not move.
func TestConfigSaveRefusalIsAllOrNothing(t *testing.T) {
	tempConfigHome(t)
	c := seedTwoProjects(t)

	if _, err := c.save(map[string]any{
		"ui":       map[string]any{"theme": "gruvbox"},
		"projects": "not a map",
	}); err == nil {
		t.Fatal("save() accepted a patch carrying a malformed wholesale value")
	}

	onDisk := newConfigService().get()
	if themeOf(onDisk) != "nord" {
		t.Errorf("the good half of a refused save landed anyway (theme = %q) — a partial apply reports success for a save that did not happen", themeOf(onDisk))
	}
	if p := projectsOnDisk(t); len(p) != 2 {
		t.Errorf("projects changed on a refused save: %#v", p)
	}
}

// TestConfigSaveStillAcceptsTheLegalWholesaleValues is the other direction. The
// refusal must not have closed the door on the calls the wholesale contract
// exists to serve — replacing the map, and emptying it with {}.
func TestConfigSaveStillAcceptsTheLegalWholesaleValues(t *testing.T) {
	tempConfigHome(t)
	c := seedTwoProjects(t)

	// A delete, expressed the only way a wholesale path can express one.
	mustSave(t, c, map[string]any{"projects": map[string]any{
		"/home/u/a": map[string]any{"label": "A", "yolo": true},
	}})
	if p := projectsOnDisk(t); len(p) != 1 {
		t.Fatalf("the delete did not land: %#v", p)
	}

	// {} still means "empty it" — which is what makes refusing null costless.
	mustSave(t, c, map[string]any{"projects": map[string]any{}})
	if p := projectsOnDisk(t); len(p) != 0 {
		t.Errorf("an explicit empty map did not empty projects: %#v", p)
	}
}

// TestConfigSaveOverTheBusReportsTheRefusal is the WIRE half: the error has to
// reach the caller as a bus error, not be swallowed into a success carrying the
// old config. Every remote writer — web, mobile, the TUI, a plugin, an agent
// through the MCP facade — sees config.save only through this door.
func TestConfigSaveOverTheBusReportsTheRefusal(t *testing.T) {
	tempConfigHome(t)
	seedTwoProjects(t)

	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	res, err := reg.handle(context.Background(), "config.save",
		json.RawMessage(`{"projects":"{\"/home/u/a\":{}}"}`))
	if err == nil {
		t.Fatalf("config.save answered a malformed wholesale value with a SUCCESS result: %s", res)
	}
	if !strings.Contains(err.Error(), "projects") {
		t.Errorf("the bus error does not name the offending path: %v", err)
	}
	if p := projectsOnDisk(t); len(p) != 2 {
		t.Errorf("a refused bus save still changed projects: %#v", p)
	}

	// And a legal save over the same door still works.
	if _, err := reg.handle(context.Background(), "config.save",
		json.RawMessage(`{"projects":{"/home/u/a":{"label":"A"}}}`)); err != nil {
		t.Fatalf("config.save refused a legal patch over the bus: %v", err)
	}
	if p := projectsOnDisk(t); len(p) != 1 {
		t.Errorf("the legal bus save did not land: %#v", p)
	}
}
