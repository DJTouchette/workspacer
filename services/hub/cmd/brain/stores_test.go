package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

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
func TestListingQuarantinesAnUnparseableFileOnce(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	bad := filepath.Join(sessionsDir(), "default.yaml")
	if err := os.WriteFile(bad, []byte("{{{ not yaml"), 0o644); err != nil {
		t.Fatal(err)
	}

	if got := listSavedSessions(); len(got) != 0 {
		t.Fatalf("expected the bad file to be skipped, got %v", got)
	}
	matches, _ := filepath.Glob(bad + ".broken-*")
	if len(matches) != 1 {
		t.Fatalf("expected exactly one quarantine copy, got %d", len(matches))
	}
	content, _ := os.ReadFile(matches[0])
	if string(content) != "{{{ not yaml" {
		t.Errorf("quarantine copy lost the original bytes: %q", content)
	}

	// Listing again must not mint a second backup — this runs on every poll.
	listSavedSessions()
	listSavedSessions()
	matches, _ = filepath.Glob(bad + ".broken-*")
	if len(matches) != 1 {
		t.Errorf("repeat listing minted %d backups, want 1", len(matches))
	}
}

func TestListLayoutsQuarantinesToo(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	if err := os.MkdirAll(layoutsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	bad := filepath.Join(layoutsDir(), "broken.yaml")
	if err := os.WriteFile(bad, []byte("{{{ not yaml"), 0o644); err != nil {
		t.Fatal(err)
	}

	listLayouts()
	if matches, _ := filepath.Glob(bad + ".broken-*"); len(matches) != 1 {
		t.Errorf("layouts got no quarantine copy (%d)", len(matches))
	}
}

// The session format version is a cross-language contract: the desktop reader
// refuses a file stamped higher than it understands, so the two writers must
// agree on what "current" is.
func TestSessionSchemaVersionMatchesContract(t *testing.T) {
	path := filepath.Join("..", "..", "..", "..", "contracts", "session-schema.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read contract fixture %s: %v", path, err)
	}
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
	Name          string       `json:"name"`
	Filename      string       `json:"filename"`
	Expect        string       `json:"expect"`
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
	raw, err := os.ReadFile(contractFixtureRel)
	if err != nil {
		t.Fatalf("read the shared fixture: %v", err)
	}
	var fx sessionFilenameFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", contractFixtureRel, err)
	}
	cases := fx.SessionFilenames.Cases
	if len(cases) == 0 {
		t.Fatalf("%s decoded to zero sessionFilenames cases — a silently empty corpus guards nothing", contractFixtureRel)
	}

	for _, c := range cases {
		t.Run(c.Name, func(t *testing.T) {
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

			got, ok := sessionFilePath(c.Filename)
			want := c.Expect == "accept"
			if ok != want {
				t.Fatalf("sessionFilePath(%q) ok=%v, want %v\n  why: %s", c.Filename, ok, want, c.Why)
			}
			if !ok {
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
		t.Run(tc.name, func(t *testing.T) {
			cfg := t.TempDir()
			t.Setenv("XDG_CONFIG_HOME", cfg)
			if err := os.MkdirAll(tc.dir(), 0o755); err != nil {
				t.Fatal(err)
			}
			token := filepath.Join(configDir(), "remote-token")
			if err := os.WriteFile(token, []byte(secret), 0o600); err != nil {
				t.Fatal(err)
			}
			link := filepath.Join(tc.dir(), "pwn.yaml")
			if err := os.Symlink(token, link); err != nil {
				t.Skipf("symlinks unavailable: %v", err)
			}

			tc.list()

			// The control: fs.read of the symlink itself is refused, and the
			// quarantine copy must not be a way around that.
			reg := registryWithCwds(t)
			if _, err := reg.handle(context.Background(), "fs.read",
				json.RawMessage(`{"path":`+jsonStr(link)+`}`)); err == nil {
				t.Fatal("fs.read of the planted symlink should be denied")
			}

			matches, _ := filepath.Glob(link + ".broken-*")
			for _, m := range matches {
				body, _ := os.ReadFile(m)
				if strings.Contains(string(body), secret) {
					t.Fatalf("%s minted %s holding the credential — fs.read of it is allowed "+
						"(a plain file in a store carve-out), so this launders remote-token", tc.name, filepath.Base(m))
				}
			}
			if len(matches) != 0 {
				t.Errorf("%s quarantined an entry that resolves outside the store (%v); it should be skipped", tc.name, matches)
			}
			// And the credential itself is untouched: a lister must not rewrite
			// what it refuses to read.
			if body, err := os.ReadFile(token); err != nil || string(body) != secret {
				t.Fatalf("remote-token was disturbed: %q %v", body, err)
			}
		})
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
	if err := os.Symlink(real, filepath.Join(layoutsDir(), "alias.yaml")); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	if got := listLayouts(); len(got) != 2 {
		t.Fatalf("an in-store symlink must still be listed; got %d layouts: %+v", len(got), got)
	}
}
