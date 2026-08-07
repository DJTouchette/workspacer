package main

import (
	"context"
	"encoding/json"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
	yaml "gopkg.in/yaml.v3"
)

func TestSlugs(t *testing.T) {
	cases := []struct {
		fn   func(string) string
		in   string
		want string
	}{
		{slugLayout, "My Layout!", "my-layout"},
		{slugLayout, "  spaced  ", "spaced"},
		{slugLayout, "a//b**c", "a-b-c"},
		{slugLayout, "!!!", "layout"}, // empty after trim → fallback
		{slugSession, "My Session", "my-session"},
		{slugSession, "keep_under-score", "keep_under-score"},
	}
	for _, c := range cases {
		if got := c.fn(c.in); got != c.want {
			t.Errorf("slug(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestLayoutsSaveListDelete(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	saved, err := saveLayout(map[string]any{
		"name":   "My Layout",
		"agents": []any{map[string]any{"name": "a", "cwd": "/tmp", "tabs": []any{}}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if saved["id"] != "my-layout" || saved["createdAt"] == "" {
		t.Fatalf("unexpected saved layout: %+v", saved)
	}
	if _, err := os.Stat(filepath.Join(dir, "workspacer", "layouts", "my-layout.yaml")); err != nil {
		t.Fatalf("layout file not written: %v", err)
	}

	// A malformed file (no agents) is ignored by list.
	_ = os.WriteFile(filepath.Join(dir, "workspacer", "layouts", "junk.yaml"), []byte("name: junk\n"), 0o644)

	list := listLayouts()
	if len(list) != 1 || list[0]["id"] != "my-layout" {
		t.Fatalf("expected only the valid layout, got %+v", list)
	}

	// remove re-slugs the id and unlinks the matching file.
	removeLayout("My Layout")
	if len(listLayouts()) != 0 {
		t.Fatal("layout should be gone after remove")
	}
}

func TestLayoutListSortsNewestFirst(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	ldir := filepath.Join(dir, "workspacer", "layouts")
	_ = os.MkdirAll(ldir, 0o755)
	_ = os.WriteFile(filepath.Join(ldir, "old.yaml"), []byte("id: old\ncreatedAt: \"2020-01-01T00:00:00.000Z\"\nagents: []\n"), 0o644)
	_ = os.WriteFile(filepath.Join(ldir, "new.yaml"), []byte("id: new\ncreatedAt: \"2024-01-01T00:00:00.000Z\"\nagents: []\n"), 0o644)

	list := listLayouts()
	if len(list) != 2 || list[0]["id"] != "new" {
		t.Fatalf("expected newest first, got %+v", list)
	}
}

func TestSavedSessionsRoundTrip(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	reg := newRegistry(newClaudemonClient("http://unused"))
	ctx := context.Background()

	// Save an agent-centric session with one global agent (excluded from count).
	params := `{"name":"Work","activeAgentId":"a1","agents":[
		{"id":"a1","tabs":[{"panes":[{},{}]}]},
		{"id":"g","global":true,"tabs":[{"panes":[{}]}]}
	]}`
	res, err := reg.handle(ctx, "sessions.save", json.RawMessage(params))
	if err != nil {
		t.Fatal(err)
	}
	var filename string
	_ = json.Unmarshal(res, &filename)
	if filename != "work.yaml" {
		t.Fatalf("filename = %q, want work.yaml", filename)
	}

	// list reports counts: 3 panes total, 1 non-global agent.
	listRes, _ := reg.handle(ctx, "sessions.list", nil)
	var list []sessionListEntry
	_ = json.Unmarshal(listRes, &list)
	if len(list) != 1 || list[0].PaneCount != 3 || list[0].AgentCount != 1 {
		t.Fatalf("unexpected list entry: %+v", list)
	}
	if list[0].Timestamp == "" {
		t.Error("save should stamp a timestamp")
	}

	// load returns the blob; a missing file returns JSON null.
	loadRes, _ := reg.handle(ctx, "sessions.load", json.RawMessage(`{"filename":"work.yaml"}`))
	var loaded map[string]any
	if err := json.Unmarshal(loadRes, &loaded); err != nil || loaded["name"] != "Work" {
		t.Fatalf("load returned %s (err %v)", loadRes, err)
	}
	missing, _ := reg.handle(ctx, "sessions.load", json.RawMessage(`{"filename":"nope.yaml"}`))
	if string(missing) != "null" {
		t.Errorf("missing session should load as null, got %s", missing)
	}

	// delete removes it.
	if _, err := reg.handle(ctx, "sessions.delete", json.RawMessage(`{"filename":"work.yaml"}`)); err != nil {
		t.Fatal(err)
	}
	listRes2, _ := reg.handle(ctx, "sessions.list", nil)
	var list2 []sessionListEntry
	_ = json.Unmarshal(listRes2, &list2)
	if len(list2) != 0 {
		t.Fatalf("session should be deleted, got %+v", list2)
	}
}

func TestPaneCountLegacyAndFlat(t *testing.T) {
	legacy := map[string]any{"tabs": []any{
		map[string]any{"panes": []any{map[string]any{}}},
		map[string]any{"panes": []any{map[string]any{}, map[string]any{}}},
	}}
	if got := paneCount(legacy); got != 3 {
		t.Errorf("legacy paneCount = %d, want 3", got)
	}
	flat := map[string]any{"panes": []any{map[string]any{}, map[string]any{}}}
	if got := paneCount(flat); got != 2 {
		t.Errorf("flat paneCount = %d, want 2", got)
	}
}

// TestSavedSessionPathContainment proves loadSavedSession/deleteSavedSession
// reject a client-supplied traversal filename instead of reading or removing a
// file outside the sessions directory (filepath.Join runs Clean, which collapses
// ".." rather than blocking it). Covers idx 14.
func TestSavedSessionPathContainment(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		t.Fatal(err)
	}

	// A secret file OUTSIDE the sessions directory (sibling of the config dir).
	// sessionsDir() == <dir>/workspacer/sessions, so ../../ lands at <dir>.
	secret := filepath.Join(dir, "secret.yaml")
	if err := os.WriteFile(secret, []byte("name: secret\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	traversal := filepath.Join("..", "..", "secret.yaml")

	// load must NOT read a file outside the sessions dir.
	if got := loadSavedSession(traversal); got != nil {
		t.Fatalf("loadSavedSession leaked out-of-dir file: %+v", got)
	}

	// delete must NOT remove a file outside the sessions dir.
	deleteSavedSession(traversal)
	if _, err := os.Stat(secret); err != nil {
		t.Fatalf("deleteSavedSession removed out-of-dir file: %v", err)
	}
}

// TestLayoutSavePathContainment is the sessions test above, for layouts — the
// store that DIDN'T have the guard. saveLayout used the raw caller id as the
// filename, so `layouts.save` with id "../config" wrote the layout over
// ~/.config/workspacer/config.yaml: themes, keybindings, budgets, gone. And
// silently, because the clobbered file still parses as YAML, so loadFromDisk
// takes it as the user's config rather than backing it up as .broken-*.
func TestLayoutSavePathContainment(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	// The real config.yaml, one directory above layouts/.
	if err := os.MkdirAll(layoutsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	original := "ui:\n  theme: everforest\n"
	if err := os.WriteFile(configPath(), []byte(original), 0o644); err != nil {
		t.Fatal(err)
	}

	if _, err := saveLayout(map[string]any{
		"id": filepath.Join("..", "config"), "name": "pwn", "agents": []any{},
	}); err == nil {
		t.Error("layouts.save with a traversal id should be refused")
	}
	if got := readFile(t, configPath()); got != original {
		t.Fatalf("layouts.save clobbered config.yaml: %q", got)
	}

	// An id that merely needs tidying is slugged, not refused — and the file it
	// lands in is the one removeLayout (which re-slugs) will unlink.
	saved, err := saveLayout(map[string]any{"id": "My Layout", "agents": []any{}})
	if err != nil {
		t.Fatal(err)
	}
	if saved["id"] != "my-layout" {
		t.Fatalf("saved id = %v, want the slug my-layout so save/remove agree", saved["id"])
	}
	removeLayout("My Layout")
	if len(listLayouts()) != 0 {
		t.Fatal("removeLayout should unlink the file saveLayout wrote")
	}
}

// Two session names can slug to the same file, so a blind write lets the second
// clobber the first. sessionService.saveSession has always guarded this; the
// brain — the DEFAULT writer under DELEGATE_CATALOG_TO_BRAIN — did not, so the
// guarded copy was the one that never ran.
func TestSaveSavedSessionDoesNotClobberADifferentSession(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	first, err := saveSavedSession("Feature: Auth", map[string]any{"name": "Feature: Auth"})
	if err != nil {
		t.Fatalf("save first: %v", err)
	}
	second, err := saveSavedSession("Feature Auth", map[string]any{"name": "Feature Auth"})
	if err != nil {
		t.Fatalf("save second: %v", err)
	}
	if first == second {
		t.Fatalf("both sessions wrote to %s — the second clobbered the first", first)
	}

	// Re-saving the FIRST one must go back to its own file, not mint a third:
	// that is what keeps an ordinary autosave stable.
	again, err := saveSavedSession("Feature: Auth", map[string]any{"name": "Feature: Auth"})
	if err != nil {
		t.Fatalf("re-save first: %v", err)
	}
	if again != first {
		t.Errorf("re-save went to %s, want its original %s", again, first)
	}
}

// The rejected-slot arm of the same loop, which is where the clobber the test
// above prevents came back in.
//
// resolveSessionFilename walks feature-auth.yaml, feature-auth-2.yaml, … until
// it finds a free or already-ours slot. When containment REFUSES a slot the arm
// answered `base + ".yaml"` — described in the comment as "let the caller fail",
// which it is only on the first iteration. From the second on it is "fall back
// to the file belonging to a DIFFERENT session", and saveSavedSession then
// re-checks that name (it passes: an ordinary file inside the store) and writes.
// The other session is destroyed with no .broken-* copy, because quarantine
// fires on a parse failure and that file parses.
//
// The refusal is reachable by design: <configDir>/sessions is a
// configStoreRoot, so planting feature-auth-2.yaml as a symlink out of the store
// is an ordinary permitted fs.write.
func TestARejectedSessionSlotDoesNotFallBackOntoAnotherSessionsFile(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	outside := t.TempDir()
	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		t.Fatal(err)
	}

	first, err := saveSavedSession("Feature: Auth", map[string]any{"name": "Feature: Auth"})
	if err != nil {
		t.Fatalf("save first: %v", err)
	}
	if first != "feature-auth.yaml" {
		t.Fatalf("precondition: expected the first session at feature-auth.yaml, got %s", first)
	}
	loot := filepath.Join(outside, "loot.yaml")
	gateSymlink(t, loot, filepath.Join(sessionsDir(), "feature-auth-2.yaml"))

	// "Feature Auth" slugs to the same base, so the loop must skip
	// feature-auth.yaml (not its session) and then meet the refused slot.
	if got, err := saveSavedSession("Feature Auth", map[string]any{"name": "Feature Auth"}); err == nil {
		t.Errorf("a refused slot must fail the save, not redirect it; wrote %s", got)
	}
	raw, err := os.ReadFile(filepath.Join(sessionsDir(), first))
	if err != nil || !strings.Contains(string(raw), "Feature: Auth") {
		t.Errorf("the OTHER session's file was overwritten: %q (%v)", raw, err)
	}
	if _, err := os.Lstat(loot); err == nil {
		t.Error("the save wrote through the symlink, out of the store")
	}
}

// A file we cannot parse is not a file we may overwrite — we cannot tell whose
// it is.
func TestSaveSavedSessionSkipsAnUnparseableFile(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	occupied := filepath.Join(sessionsDir(), "default.yaml")
	if err := os.WriteFile(occupied, []byte("{{{ not yaml"), 0o644); err != nil {
		t.Fatal(err)
	}

	name, err := saveSavedSession("Default", map[string]any{"name": "Default"})
	if err != nil {
		t.Fatalf("save: %v", err)
	}
	if name == "default.yaml" {
		t.Fatal("wrote over a file it could not identify")
	}
	raw, err := os.ReadFile(occupied)
	if err != nil || string(raw) != "{{{ not yaml" {
		t.Errorf("the unreadable file was modified: %q, %v", raw, err)
	}
}

// Both listers skip a file they cannot parse, so a corrupt default.yaml simply
// vanishes from the list and the next autosave writes over it. The copy aside is
// the backstop.
// backupsOf counts the .broken-* siblings of `path` by a LITERAL prefix scan.
// Deliberately not filepath.Glob — the names below contain glob metacharacters
// on purpose, and an oracle built out of the same primitive as the code under
// test cannot see the bug in it.
func backupsOf(t *testing.T, path string) []string {
	t.Helper()
	entries, err := os.ReadDir(filepath.Dir(path))
	if err != nil {
		return nil
	}
	prefix := filepath.Base(path) + ".broken-"
	var out []string
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), prefix) {
			out = append(out, filepath.Join(filepath.Dir(path), e.Name()))
		}
	}
	return out
}

// The once-guard, over names a CALLER can choose. <configDir>/sessions and
// <configDir>/layouts are configStoreRoots, so `fs.write` of a file called
// "loot[.yaml" is an ordinary permitted write — and the guard used to be
// filepath.Glob(path + ".broken-*"), which INTERPRETS that name:
//
//	'[' → ErrBadPattern, no matches, so every list call mints another full copy
//	      of the file. A UI that polls sessions.list fills the disk.
//	'*' → the pattern matches some OTHER file's backup, so the corrupt file is
//	      never backed up at all — the data loss the function exists to prevent.
//
// The sleeps matter too: the backup name has millisecond resolution, so three
// back-to-back list calls used to collide on one filename and the guard looked
// like it was working even when it had been deleted outright.
func TestListingQuarantinesAnUnparseableFileOnce(t *testing.T) {
	for _, name := range []string{"default.yaml", "loot[.yaml", "a*.yaml", "q?.yaml"} {
		t.Run(name, func(t *testing.T) {
			dir := t.TempDir()
			t.Setenv("XDG_CONFIG_HOME", dir)
			if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
				t.Fatal(err)
			}
			// A decoy backup belonging to a DIFFERENT file, so a pattern with a
			// '*' in it has something wrong to match.
			if err := os.WriteFile(filepath.Join(sessionsDir(), "zzz.yaml.broken-2020-01-01T00-00-00.000"), []byte("decoy"), 0o600); err != nil {
				t.Fatal(err)
			}
			bad := filepath.Join(sessionsDir(), name)
			if err := os.WriteFile(bad, []byte("{{{ not yaml"), 0o644); err != nil {
				t.Fatal(err)
			}

			if got := listSavedSessions(); len(got) != 0 {
				t.Fatalf("expected the bad file to be skipped, got %v", got)
			}
			matches := backupsOf(t, bad)
			if len(matches) != 1 {
				t.Fatalf("expected exactly one quarantine copy of %q, got %d %v", name, len(matches), matches)
			}
			content, _ := os.ReadFile(matches[0])
			if string(content) != "{{{ not yaml" {
				t.Errorf("quarantine copy lost the original bytes: %q", content)
			}

			// Listing again must not mint a second backup — this runs on every
			// poll. The sleep is what makes the assertion real: without it the
			// millisecond-resolution names collide and a deleted guard passes.
			for i := 0; i < 4; i++ {
				time.Sleep(2 * time.Millisecond)
				listSavedSessions()
			}
			if matches := backupsOf(t, bad); len(matches) != 1 {
				t.Errorf("repeat listing minted %d backups of %q, want 1: %v", len(matches), name, matches)
			}
		})
	}
}

func TestListLayoutsQuarantinesToo(t *testing.T) {
	for _, name := range []string{"broken.yaml", "b[roken.yaml"} {
		t.Run(name, func(t *testing.T) {
			dir := t.TempDir()
			t.Setenv("XDG_CONFIG_HOME", dir)
			if err := os.MkdirAll(layoutsDir(), 0o755); err != nil {
				t.Fatal(err)
			}
			bad := filepath.Join(layoutsDir(), name)
			if err := os.WriteFile(bad, []byte("{{{ not yaml"), 0o644); err != nil {
				t.Fatal(err)
			}

			listLayouts()
			if matches := backupsOf(t, bad); len(matches) != 1 {
				t.Errorf("layouts got %d quarantine copies of %q, want 1", len(matches), name)
			}
			time.Sleep(2 * time.Millisecond)
			listLayouts()
			if matches := backupsOf(t, bad); len(matches) != 1 {
				t.Errorf("repeat listing minted %d backups of %q, want 1", len(matches), name)
			}
		})
	}
}

// The session format version is a cross-language contract: the desktop reader
// refuses a file stamped higher than it understands, so the two writers must
// agree on what "current" is.
func TestSessionSchemaVersionMatchesContract(t *testing.T) {
	const path = "contracts/session-schema.json"
	data := mustReadRepoFile(t, "contracts", "session-schema.json")
	var fixture struct {
		Version int `json:"version"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parse contract fixture: %v", err)
	}
	if fixture.Version != sessionSchemaVersion {
		t.Errorf("sessionSchemaVersion = %d, contract says %d", sessionSchemaVersion, fixture.Version)
	}
}

func TestSaveSavedSessionStampsTheVersion(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	name, err := saveSavedSession("Default", map[string]any{"name": "Default"})
	if err != nil {
		t.Fatalf("save: %v", err)
	}
	raw, err := os.ReadFile(filepath.Join(sessionsDir(), name))
	if err != nil {
		t.Fatal(err)
	}
	var out map[string]any
	if err := yaml.Unmarshal(raw, &out); err != nil {
		t.Fatal(err)
	}
	if got, _ := out["schemaVersion"].(int); got != sessionSchemaVersion {
		t.Errorf("schemaVersion = %v, want %d", out["schemaVersion"], sessionSchemaVersion)
	}
}

// ---------------------------------------------------------------------------
// contracts/path-containment-cases.json → sessionFilenames
//
// sessions.load / sessions.save / sessions.delete are the FOURTH copy of path
// containment and the one nothing classified: capspec's pathVerbPrefixes do not
// cover sessions.*, its pathishParams did not include `filename`, and the
// corpus's `methods` block cannot hold it (that set must equal capspec.PathParam
// exactly, and PathParam entries are absolute paths — a session basename is
// not). So the two copies drifted: Go required a bare basename while the desktop
// used path.resolve + startsWith, a purely lexical check that accepted any
// multi-segment name under the sessions dir and therefore read and unlinked
// through a directory symlink. This loader and the one in
// apps/desktop/src/main/services/sessionService.test.ts read the same block.
// ---------------------------------------------------------------------------

type sessionFilenameCase struct {
	Name     string `json:"name"`
	Filename string `json:"filename"`
	Expect   string `json:"expect"`
	// RefusedBy is the RIGHT-REASON half of a refusal, named from the fixture's
	// `vocabulary.sessionRefuseReasons`. `expect: refuse` on its own is
	// satisfied by a refusal for ANY reason — including a resolver that refuses
	// everything, which is the failure the accept cases are the only defence
	// against — and it hid something specific here: the case named "a
	// multi-segment name that traverses a symlink out of the sessions dir" is
	// refused by the BASENAME rule and never reaches the symlink at all. Naming
	// the reason says which of the two rules each case actually exercises, and
	// keeps the one case that exercises the second rule from being deleted by
	// accident.
	RefusedBy     string       `json:"refusedBy"`
	ResolvesTo    string       `json:"resolvesTo"`
	NeedsSymlinks bool         `json:"needsSymlinks"`
	Tree          contractTree `json:"tree"`
	Why           string       `json:"why"`
}

type sessionFilenameFixture struct {
	SessionFilenames struct {
		Cases []sessionFilenameCase `json:"cases"`
	} `json:"sessionFilenames"`
}

func TestSessionFilenameContractCases(t *testing.T) {
	raw := readContractFixtureBytes(t)
	var fx sessionFilenameFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", contractFixtureRel, err)
	}
	cases := fx.SessionFilenames.Cases
	if len(cases) == 0 {
		t.Fatalf("%s decoded to zero sessionFilenames cases — a silently empty corpus guards nothing", contractFixtureRel)
	}

	// len(cases) is what the fixture CARRIES; the tally is what this loop
	// executed. A needsSymlinks case skips itself below, and on a host without
	// symlink privilege that silently turns the sessions-store oracle into zero
	// assertions inside a green package.
	var tally sweepguard.Tally

	for _, c := range cases {
		t.Run(c.Name, func(t *testing.T) {
			t.Cleanup(func() {
				if t.Skipped() {
					if c.NeedsSymlinks {
						tally.Skip("needsSymlinks")
					} else {
						tally.Skip("unexplained (the case declares no host requirement)")
					}
				}
			})
			sandbox, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			configHome := filepath.Join(sandbox, "config")
			for _, d := range []string{filepath.Join("config", "workspacer", "sessions"), "outside"} {
				if err := os.MkdirAll(filepath.Join(sandbox, d), 0o755); err != nil {
					t.Fatal(err)
				}
			}
			t.Setenv("XDG_CONFIG_HOME", configHome)
			t.Setenv("APPDATA", configHome)

			for _, d := range c.Tree.Dirs {
				if err := os.MkdirAll(filepath.Join(sandbox, d), 0o755); err != nil {
					t.Fatal(err)
				}
			}
			for rel, body := range c.Tree.Files {
				full := filepath.Join(sandbox, rel)
				if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(full, []byte(body), 0o644); err != nil {
					t.Fatal(err)
				}
			}
			for rel, dest := range c.Tree.Symlinks {
				full := filepath.Join(sandbox, rel)
				if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.Symlink(filepath.Join(sandbox, dest), full); err != nil {
					if c.NeedsSymlinks {
						t.Skipf("needsSymlinks: cannot create symlinks here: %v", err)
					}
					t.Fatal(err)
				}
			}

			// Past the only skip gate (the symlink leg of the tree above).
			tally.Ran(c.Expect)

			got, ok := sessionFilePath(c.Filename)
			want := c.Expect == "accept"
			if ok != want {
				t.Fatalf("sessionFilePath(%q) ok=%v, want %v\n  why: %s", c.Filename, ok, want, c.Why)
			}
			if !ok {
				// THE RIGHT REASON, computed by an oracle that shares no code
				// with sessionFilePath.
				if got := sessionRefusalReason(c.Filename); got != c.RefusedBy {
					t.Fatalf("refused for the WRONG REASON: the oracle says %q, the fixture says %q\n  filename: %q\n  why: %s",
						got, c.RefusedBy, c.Filename, c.Why)
				}
				// A refusal must also be inert at the store level: the two verbs a
				// bus caller can reach must neither return the file's contents nor
				// delete it. Asking sessionFilePath alone would leave a copy that
				// answered "no" and then opened the file anyway completely green.
				if s := loadSavedSession(c.Filename); len(s) != 0 {
					t.Fatalf("loadSavedSession(%q) returned data for a refused filename: %v", c.Filename, s)
				}
				deleteSavedSession(c.Filename)
				for rel := range c.Tree.Files {
					if _, err := os.Stat(filepath.Join(sandbox, rel)); err != nil {
						t.Fatalf("deleteSavedSession(%q) removed %s, which it refused to resolve: %v", c.Filename, rel, err)
					}
				}
				return
			}
			if c.ResolvesTo == "" {
				t.Fatalf("an accept case must pin `resolvesTo` — the returned string is what gets opened")
			}
			if wantPath := filepath.Join(sandbox, c.ResolvesTo); got != wantPath {
				t.Fatalf("sessionFilePath(%q) = %q, want %q\n  why: %s", c.Filename, got, wantPath, c.Why)
			}
		})
	}

	// Accepts and refuses counted apart: a sessionFilePath that returned ok=false
	// unconditionally satisfies every refuse case in this block, and one that
	// returned ok=true satisfies every accept case. Only running both classes
	// says anything.
	if err := tally.RequireCorpus("the sessionFilenames corpus", sessionFilenameFloor, 1, 1); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

// sessionRefusalReason is the INDEPENDENT oracle for why a session filename must
// be refused, written from the two rules rather than from the implementation:
// rule 1 is "a plain basename", rule 2 is "canonicalizes inside the sessions
// dir". It shares no code with sessionFilePath — rule 2 is answered with
// EvalSymlinks — so a copy that collapsed the two rules into one cannot satisfy
// it, and a case whose name claims a symlink escape but which is really refused
// for its separators is named as such.
func sessionRefusalReason(filename string) string {
	if strings.TrimSpace(filename) == "" || filename == "." || filename == ".." ||
		filepath.IsAbs(filename) || filename != filepath.Base(filename) {
		return "not-a-basename"
	}
	real, err := filepath.EvalSymlinks(filepath.Join(sessionsDir(), filename))
	if err != nil {
		return "unresolvable (the fixture declares no such reason — a case that lands here is one nothing can attribute)"
	}
	dir, err := filepath.EvalSymlinks(sessionsDir())
	if err != nil {
		return "unresolvable sessions dir"
	}
	if real == dir || strings.HasPrefix(real, strings.TrimSuffix(dir, string(filepath.Separator))+string(filepath.Separator)) {
		return "inside the sessions dir (so this case is not refused by either rule, and the fixture is wrong to expect a refusal)"
	}
	return "escapes-sessions-dir"
}

// A store lister builds its own paths — `<storeDir>/<readdir entry>` — and those
// are DERIVED paths, which the same rule covers as the ones a caller names
// (BINDING DECISION 2). The entry name is a bare basename, so nothing can escape
// textually; a SYMLINK named like a store file is what escapes, and
// <configDir>/layouts and <configDir>/sessions are precisely the two directories
// a bus caller may write into (they are configStoreRoots).
//
// Unguarded, the leak was not just "the lister reads it": quarantineUnreadable
// COPIED the bytes of whatever would not parse to `<name>.broken-<ts>`, an
// ordinary file inside the same carve-out, with a basename that is not a
// credential — so it passed the secret gate and `fs.read` handed it back.
// remote-token in, TRUSTED bus connection out, through two allowed calls.
func TestStoreListersDoNotReadThroughASymlinkOutOfTheStore(t *testing.T) {
	const secret = "wks_remote_TRUSTED_PROMOTION_TOKEN"

	for _, tc := range []struct {
		name string
		dir  func() string
		list func()
	}{
		{"layouts.list", layoutsDir, func() { listLayouts() }},
		{"sessions.list", sessionsDir, func() { listSavedSessions() }},
	} {
		for _, victim := range []struct {
			label string
			// rel is where the victim file lives, relative to the config dir.
			rel string
		}{
			// The credential itself, one directory up from the store.
			{"the config dir's own remote-token", "remote-token"},
			// A PREFIX COLLISION with the store's name. The secret gate denies
			// <configDir>/sessions-backup one layer up (the corpus has that
			// case), but storeEntryPath's containment is a separate comparison
			// with no case of its own: `strings.HasPrefix(canonical, dir)` — no
			// separator boundary — reads straight through this one.
			{"a config-dir sibling whose name starts with the store's", "STORE-backup/loot.yaml"},
		} {
			t.Run(tc.name+"/"+victim.label, func(t *testing.T) {
				cfg := t.TempDir()
				t.Setenv("XDG_CONFIG_HOME", cfg)
				if err := os.MkdirAll(tc.dir(), 0o755); err != nil {
					t.Fatal(err)
				}
				rel := strings.ReplaceAll(victim.rel, "STORE", filepath.Base(tc.dir()))
				token := filepath.Join(configDir(), filepath.FromSlash(rel))
				if err := os.MkdirAll(filepath.Dir(token), 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(token, []byte(secret), 0o600); err != nil {
					t.Fatal(err)
				}
				link := filepath.Join(tc.dir(), "pwn.yaml")
				gateSymlink(t, token, link)

				before := treeSnapshot(t, cfg)
				tc.list()
				after := treeSnapshot(t, cfg)

				// The control: fs.read of the symlink itself is refused, and no
				// derived artefact may be a way around that.
				reg := registryWithCwds(t)
				if _, err := reg.handle(context.Background(), "fs.read",
					json.RawMessage(`{"path":`+jsonStr(link)+`}`)); err == nil {
					t.Fatal("fs.read of the planted symlink should be denied")
				}

				// WHEREVER it landed. The earlier version of this test globbed
				// `link + ".broken-*"` — beside the SYMLINK, which is where an
				// uncanonicalized lister would put the copy. A lister that
				// canonicalizes but does not CONTAIN lands it beside the file the
				// link resolved to, in a directory that glob never looked at, so
				// deleting storeEntryPath's isWithin left the whole suite green
				// while sessions.list wrote a byte-for-byte copy of any
				// unparseable file next to the victim. Diff the whole tree.
				for path, body := range after {
					if _, existed := before[path]; existed {
						continue
					}
					t.Errorf("%s created %s (%d bytes) — a read-only capability must not write anything",
						tc.name, path, len(body))
					if strings.Contains(body, secret) {
						t.Errorf("  …and it holds the credential verbatim: this launders %s into a readable file", rel)
					}
				}
				// And the victim itself is untouched: a lister must not rewrite
				// what it refuses to read.
				if body, err := os.ReadFile(token); err != nil || string(body) != secret {
					t.Fatalf("%s was disturbed: %q %v", rel, body, err)
				}
			})
		}
	}
}

// treeSnapshot maps every regular file under root to its contents. Symlinks are
// recorded by their link body rather than followed, so a planted link does not
// make the snapshot itself read the victim.
func treeSnapshot(t *testing.T, root string) map[string]string {
	t.Helper()
	out := map[string]string{}
	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return nil //nolint:nilerr // an unreadable subtree is simply not in the snapshot
		}
		if d.Type()&fs.ModeSymlink != 0 {
			dest, _ := os.Readlink(path)
			out[path] = "-> " + dest
			return nil
		}
		body, _ := os.ReadFile(path)
		out[path] = string(body)
		return nil
	})
	if err != nil {
		t.Fatalf("snapshot %s: %v", root, err)
	}
	return out
}

// The write and delete legs of the same containment. sessionFilePath is shared by
// load/save/delete, but only the READ legs had cases: saveSavedSession could drop
// back to a bare filepath.Join and deleteSavedSession to a lexical prefix compare
// with the whole suite green. Both of those unlink or overwrite through a planted
// symlink; the store dir is one a bus caller can fs.write into.
func TestSessionWriteAndDeleteRefuseAnEntryThatResolvesOutOfTheStore(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	// A prefix collision, so a HasPrefix-without-separator containment reads it
	// as "inside the sessions dir".
	victimDir := filepath.Join(configDir(), "sessions-backup")
	if err := os.MkdirAll(victimDir, 0o755); err != nil {
		t.Fatal(err)
	}
	victim := filepath.Join(victimDir, "loot.yaml")
	if err := os.WriteFile(victim, []byte("name: precious\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(sessionsDir(), "precious.yaml")
	gateSymlink(t, victim, link)

	if got := loadSavedSession("precious.yaml"); got != nil {
		t.Errorf("sessions.load read through the symlink: %v", got)
	}

	// saveSavedSession picks its own filename via resolveSessionFilename, which
	// re-reads the store; the name "precious" slugs onto the planted link. The
	// WRITE leg has to reach the same verdict as the two read legs — its own
	// comment says "the three paths can never disagree about what a legal session
	// file is" — so it must refuse rather than quietly land somewhere.
	if got, err := saveSavedSession("precious", map[string]any{"name": "precious"}); err == nil {
		t.Errorf("sessions.save accepted an entry that resolves out of the store, writing %q", got)
	}
	// writeFileAtomic renames over its destination, so an unguarded write does
	// not corrupt the victim — it REPLACES the planted link with a real file, and
	// that is the observable difference. (The victim is checked too: a
	// non-atomic writer, or an os.Remove first, would go straight through.)
	if st, err := os.Lstat(link); err != nil || st.Mode()&os.ModeSymlink == 0 {
		t.Errorf("sessions.save wrote to the planted entry (it is no longer a symlink): %v %v", st, err)
	}
	if body, err := os.ReadFile(victim); err != nil || string(body) != "name: precious\n" {
		t.Errorf("sessions.save wrote through the symlink: %q %v", body, err)
	}

	deleteSavedSession("precious.yaml")
	if _, err := os.Stat(victim); err != nil {
		t.Errorf("sessions.delete unlinked the file outside the store: %v", err)
	}
}

// The layout store's resolver is the FIFTH copy of this predicate and the only
// one that never canonicalized: layoutFilePath compared filepath.Dir(full) to
// Clean(layoutsDir()), which is a purely lexical answer and cannot see a symlink
// entry — so the read path (storeEntryPath) and the write/delete path disagreed
// about what a legal entry is, inside one store.
func TestLayoutWriteAndDeleteRefuseAnEntryThatResolvesOutOfTheStore(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	if err := os.MkdirAll(layoutsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	victim := filepath.Join(configDir(), "config.yaml")
	if err := os.WriteFile(victim, []byte("ui: {}\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, victim, filepath.Join(layoutsDir(), "pwn.yaml"))

	if _, err := saveLayout(map[string]any{"id": "pwn", "agents": []any{}}); err == nil {
		t.Error("layouts.save through an entry that resolves out of the store should be refused")
	}
	if body, _ := os.ReadFile(victim); string(body) != "ui: {}\n" {
		t.Errorf("layouts.save clobbered %s through the symlink: %q", victim, body)
	}
	removeLayout("pwn")
	if _, err := os.Stat(victim); err != nil {
		t.Errorf("layouts.delete unlinked the file outside the store: %v", err)
	}
}

// The two "unverifiable → refuse" arms the store guards still had uncovered:
// storeEntryPath's `canonicalizePath error → skip` and layoutFilePath's
// `canonicalizePath error → refuse`. Both carry an explicit posture comment
// ("unverifiable → skip, same posture as the fs.* guard"), the coverage profile
// reported both blocks with zero executions, and both could be changed to return
// the LEXICAL filepath.Join with ok=true / a nil error against a green package.
// Every equivalent posture on this surface — canonicalRoot's discard,
// assertPathAllowed's refusal, pathIsSecret's unverifiable target — is pinned;
// these were the last two that were not.
//
// A self-referential symlink is the cheapest unresolvable entry: the walk hits
// maxLinkHops rather than any filesystem verdict, so the assertion does not
// depend on which errno this platform reports.
func TestStoreGuardsRefuseAnEntryTheyCannotResolve(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	for _, d := range []string{sessionsDir(), layoutsDir()} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	sessionCycle := filepath.Join(sessionsDir(), "cycle.yaml")
	gateSymlink(t, sessionCycle, sessionCycle)
	layoutCycle := filepath.Join(layoutsDir(), "cycle.yaml")
	if err := os.Symlink(layoutCycle, layoutCycle); err != nil {
		t.Fatal(err)
	}

	if got, ok := storeEntryPath(sessionsDir(), "cycle.yaml"); ok {
		t.Errorf("storeEntryPath accepted an entry it could not resolve, answering %q", got)
	}
	if got, err := layoutFilePath("cycle"); err == nil {
		t.Errorf("layoutFilePath accepted an id it could not resolve, answering %q", got)
	}
	// And the write leg refuses rather than renaming a fresh file over the
	// unresolvable entry, which is what a lexical fallback would do.
	if _, err := saveLayout(map[string]any{"id": "cycle", "name": "C"}); err == nil {
		t.Error("saveLayout must refuse a layout id that does not resolve")
	}
	st, err := os.Lstat(layoutCycle)
	if err != nil || st.Mode()&os.ModeSymlink == 0 {
		t.Errorf("saveLayout replaced the unresolvable entry with a regular file: %v", err)
	}
}

// The floor: a symlink that stays INSIDE the store is an ordinary entry and must
// still be listed. Refusing every symlink would satisfy the test above.
func TestStoreListersFollowASymlinkThatStaysInsideTheStore(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	if err := os.MkdirAll(layoutsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	real := filepath.Join(layoutsDir(), "real.yaml")
	body := "id: real\nname: Real\ncreatedAt: '2026-01-01T00:00:00.000Z'\nagents: []\n"
	if err := os.WriteFile(real, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, real, filepath.Join(layoutsDir(), "alias.yaml"))
	if got := listLayouts(); len(got) != 2 {
		t.Fatalf("an in-store symlink must still be listed; got %d layouts: %+v", len(got), got)
	}
}
