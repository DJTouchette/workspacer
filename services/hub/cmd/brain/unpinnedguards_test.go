package main

// Guards whose mutants survived the whole package.
//
// Every test here corresponds to a line of shipping code that carries a comment
// explaining why it exists, and to a mutation that deleted or inverted that line
// with `go test ./...` staying 100% green. A guard nobody's test can kill is a
// comment.

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// sandboxHome points HOME and the config dir at a fresh tree and returns both.
func sandboxHome(t *testing.T) (home, cfgHome string) {
	t.Helper()
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	home = filepath.Join(sandbox, "home")
	cfgHome = filepath.Join(sandbox, "config")
	if err := os.MkdirAll(home, 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv("XDG_CONFIG_HOME", cfgHome)
	t.Setenv("APPDATA", cfgHome)
	resetCwdCacheForTest()
	return home, cfgHome
}

// ── library.remove: the basename gate at the CALL SITE ───────────────────────

// assertPlainBasename is the ONLY barrier in front of removeLibrary's
// os.RemoveAll, and the derived-path guard structurally cannot see past it:
// filepath.Join(claudeSkillsDir(cwd), "..") CLEANS to <cwd>/.claude, which is
// itself a declared entry of libraryItemDirs, so containsPath matches on
// equality and assertLibraryItemPath returns "allowed".
//
// The function was pinned; its WIRING was not. TestClaudeScopeIdsAreRealBasenames
// drives its hostile-id loop through library.SAVE only, so replacing the call in
// removeLibrary with `name := id` left the whole package green while one bus
// call — library.remove {scope:"claude", kind:"skill", id:".."} — os.RemoveAll'd
// the project's entire `.claude` tree: every skill, every subagent, every custom
// command, and settings.local.json (the permission allowlist).
func TestClaudeScopeRemoveRefusesAnIdThatIsNotABasename(t *testing.T) {
	sandboxHome(t)
	proj, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	reg := registryWithCwd(t, proj)

	seed := func() []string {
		paths := []string{
			filepath.Join(proj, ".claude", "settings.local.json"),
			filepath.Join(proj, ".claude", "agents", "reviewer.md"),
			filepath.Join(proj, ".claude", "skills", "real-skill", "SKILL.md"),
			filepath.Join(proj, ".claude", "commands", "ship.md"),
		}
		for _, p := range paths {
			if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(p, []byte("x"), 0o644); err != nil {
				t.Fatal(err)
			}
		}
		return paths
	}

	for _, bad := range []struct{ kind, id string }{
		{"skill", ".."},
		{"skill", "."},
		{"skill", "../.."},
		{"skill", "a/b"},
		{"agent", ".."},
		{"command", ".."},
	} {
		t.Run(bad.kind+"/"+bad.id, func(t *testing.T) {
			planted := seed()
			raw, _ := json.Marshal(map[string]string{
				"scope": "claude", "kind": bad.kind, "id": bad.id, "cwd": proj,
			})
			if _, err := reg.handle(context.Background(), "library.remove", raw); err != nil {
				t.Fatalf("library.remove returned an error rather than a no-op: %v", err)
			}
			for _, p := range planted {
				if _, err := os.Stat(p); err != nil {
					t.Fatalf("library.remove{id:%q} DESTROYED %s — assertPlainBasename is the only barrier in front of os.RemoveAll and it is not being applied at this call site", bad.id, p)
				}
			}
		})
	}

	// The floor: a real basename still removes the item it names, or the loop
	// above is satisfied by a library.remove that does nothing at all.
	seed()
	raw, _ := json.Marshal(map[string]string{
		"scope": "claude", "kind": "skill", "id": "real-skill", "cwd": proj,
	})
	if _, err := reg.handle(context.Background(), "library.remove", raw); err != nil {
		t.Fatalf("library.remove of a legitimate id: %v", err)
	}
	if _, err := os.Stat(filepath.Join(proj, ".claude", "skills", "real-skill")); err == nil {
		t.Fatal("floor: library.remove of a real basename did not remove the skill")
	}
}

// ── library.save scope="claude": its own, second cwd guard ───────────────────

// saveLibraryClaude holds a SECOND assertPathAllowed call site inside the same
// method, and the corpus's `methods` block hardcodes params {"scope":"project"},
// so both fixture sweeps dispatch only into saveLibrary. Widening this one from
// workspaceRoots to browseRoots left every hub package green while library.save
// wrote an attacker-authored SUBAGENT DEFINITION — prompt and tool instructions
// Claude loads and executes — into `<any dir under $HOME>/.claude/agents/`.
func TestClaudeScopeSaveIsConfinedToTheWorkspaceNotTheHomeTree(t *testing.T) {
	home, _ := sandboxHome(t)
	// A directory under $HOME that no agent runs in: inside the BROWSE roots,
	// outside the WORKSPACE roots. That is the entire distinction.
	victim := filepath.Join(home, "victim-project")
	if err := os.MkdirAll(victim, 0o755); err != nil {
		t.Fatal(err)
	}
	live, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	reg := registryWithCwd(t, live)

	// Floor first: the LIVE agent cwd is writable, so a refusal below is about
	// the root set and not about claude-scope saves being broken.
	raw, _ := json.Marshal(map[string]any{
		"scope": "claude", "kind": "agent", "id": "ok", "title": "ok",
		"body": "hello", "cwd": live,
	})
	if _, err := reg.handle(context.Background(), "library.save", raw); err != nil {
		t.Fatalf("floor: a claude-scope save into a live agent cwd must be allowed: %v", err)
	}

	raw, _ = json.Marshal(map[string]any{
		"scope": "claude", "kind": "agent", "id": "pwned", "title": "pwned",
		"body": "ATTACKER-CONTROLLED SUBAGENT DEFINITION", "cwd": victim,
	})
	if _, err := reg.handle(context.Background(), "library.save", raw); err == nil {
		t.Fatal("library.save{scope:claude} into a home-tree directory with no live agent was ALLOWED — the claude leg is using the BROWSE roots, and its payload is a subagent definition Claude executes")
	}
	if _, err := os.Stat(filepath.Join(victim, ".claude", "agents", "pwned.md")); err == nil {
		t.Fatal("the refused save still planted the file")
	}
}

// ── libraryItemDirs: the global store must be RESOLVED ───────────────────────

// The comment above that line states the property outright — "configDir() itself
// is routinely a symlinked path ... so a lexical comparison against it would
// reject every global item" — and every existing test points XDG_CONFIG_HOME at
// a plain t.TempDir(), where the resolved and lexical spellings coincide. So the
// branch was never exercised in the shape it exists for, and reverting it to the
// lexical join left the package green while library.list returned ZERO global
// items and library.save of a global item was refused, on exactly the platforms
// the comment names (macOS /var -> /private/var, an XDG dir on a linked volume,
// a home behind an automounter).
func TestGlobalLibraryItemsSurviveASymlinkedConfigDir(t *testing.T) {
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	real := filepath.Join(sandbox, "real-config")
	link := filepath.Join(sandbox, "cfg")
	if err := os.MkdirAll(real, 0o755); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, real, link)
	t.Setenv("XDG_CONFIG_HOME", link)
	t.Setenv("APPDATA", link)
	t.Setenv("HOME", filepath.Join(sandbox, "home"))
	t.Setenv("USERPROFILE", filepath.Join(sandbox, "home"))
	resetCwdCacheForTest()

	if !strings.HasPrefix(libraryGlobalDir(), link) {
		t.Fatalf("the sandbox did not put the global store behind the link: %s", libraryGlobalDir())
	}
	if err := os.MkdirAll(libraryGlobalDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	item := filepath.Join(libraryGlobalDir(), "note.md")
	if err := os.WriteFile(item, []byte("---\ntitle: Note\nkind: prompt\n---\n\nbody\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	proj, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	reg := registryWithCwd(t, proj)
	raw, _ := json.Marshal(map[string]string{"cwd": proj})
	res, err := reg.handle(context.Background(), "library.list", raw)
	if err != nil {
		t.Fatalf("library.list: %v", err)
	}
	var items []map[string]any
	if err := json.Unmarshal(res, &items); err != nil {
		t.Fatal(err)
	}
	found := false
	for _, it := range items {
		if it["title"] == "Note" {
			found = true
		}
	}
	if !found {
		t.Fatalf("the global library item vanished when the config dir is reached through a symlink; got %d item(s) — libraryItemDirs must CANONICALIZE the global store", len(items))
	}
}

// ── BINDING DECISION 2 at the store call sites ───────────────────────────────

// The corpus's `checkUse` block says the guard's ANSWER is what every call site
// hands the filesystem, and TestGuardedHandlersOpenTheCanonicalPathTheyValidated
// pins that for fs.*/search.project. The store legs had nothing: ten independent
// mutations that swapped the guard's canonical path for the unresolved join all
// survived the whole package.
//
// Four of them are observably non-equivalent right now, because os.Remove and
// os.Rename do NOT follow the final symlink while canonicalizePath does. With an
// ordinary in-store symlink `alias.yaml -> target.yaml`, the shipped code deletes
// (and rewrites) the file the guard resolved to; the mutant deletes the LINK and
// leaves the file — the same one-string invariant, on a different verb.
func TestStoreWriteAndDeleteLegsUseTheGuardsAnswer(t *testing.T) {
	_, _ = sandboxHome(t)

	t.Run("removeLayout deletes what layoutFilePath resolved to", func(t *testing.T) {
		dir := layoutsDir()
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(dir, "target.yaml")
		link := filepath.Join(dir, "alias.yaml")
		if err := os.WriteFile(target, []byte("id: target\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		gateSymlink(t, target, link)
		removeLayout("alias")
		if _, err := os.Stat(target); err == nil {
			t.Error("removeLayout deleted the LINK and left the file layoutFilePath validated — check-path and opened-path are two different strings again")
		}
		if _, err := os.Lstat(link); err != nil {
			t.Error("removeLayout removed the link itself; the guard's answer was the target")
		}
	})

	t.Run("deleteSavedSession deletes what sessionFilePath resolved to", func(t *testing.T) {
		dir := sessionsDir()
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(dir, "target.yaml")
		link := filepath.Join(dir, "alias.yaml")
		if err := os.WriteFile(target, []byte("name: target\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		gateSymlink(t, target, link)
		deleteSavedSession("alias.yaml")
		if _, err := os.Stat(target); err == nil {
			t.Error("deleteSavedSession deleted the LINK and left the file sessionFilePath validated")
		}
		if _, err := os.Lstat(link); err != nil {
			t.Error("deleteSavedSession removed the link itself; the guard's answer was the target")
		}
	})

	// The fourth leg the comment above counts. It was missing: this test shipped
	// with three subtests, so layouts.save could go on renaming a temp file over
	// the LINK — destroying the alias and leaving the file layoutFilePath
	// validated untouched — while its three siblings were all pinned. Same verb
	// as saveSavedSession's, same guard shape, opposite coverage.
	t.Run("saveLayout writes through to the resolved file", func(t *testing.T) {
		dir := layoutsDir()
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(dir, "ltarget.yaml")
		link := filepath.Join(dir, "lalias.yaml")
		if err := os.WriteFile(target, []byte("id: lalias\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		gateSymlink(t, target, link)
		if _, err := saveLayout(map[string]any{"id": "lalias", "name": "L"}); err != nil {
			t.Fatalf("saveLayout: %v", err)
		}
		st, err := os.Lstat(link)
		if err != nil {
			t.Fatal(err)
		}
		if st.Mode()&os.ModeSymlink == 0 {
			t.Error("the atomic write REPLACED the link with a regular file instead of writing the file layoutFilePath validated")
		}
		raw, err := os.ReadFile(target)
		if err != nil || !strings.Contains(string(raw), "name: L") {
			t.Errorf("the resolved target was not written: %v / %q", err, raw)
		}
	})

	t.Run("saveSavedSession writes through to the resolved file", func(t *testing.T) {
		dir := sessionsDir()
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		// The name the slug produces IS the link's basename, so the write leg
		// lands on the link and has to follow it exactly as the read leg does.
		target := filepath.Join(dir, "wtarget.yaml")
		link := filepath.Join(dir, "walias.yaml")
		if err := os.WriteFile(target, []byte("name: walias\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		gateSymlink(t, target, link)
		if _, err := saveSavedSession("walias", map[string]any{"name": "walias", "agents": []any{}}); err != nil {
			t.Fatalf("saveSavedSession: %v", err)
		}
		st, err := os.Lstat(link)
		if err != nil {
			t.Fatal(err)
		}
		if st.Mode()&os.ModeSymlink == 0 {
			t.Error("the atomic write REPLACED the link with a regular file instead of writing the file sessionFilePath validated")
		}
		raw, err := os.ReadFile(target)
		if err != nil || !strings.Contains(string(raw), "schemaVersion") {
			t.Errorf("the resolved target was not written: %v / %q", err, raw)
		}
	})
}

// ── fs.listDir: the picker's opening call ────────────────────────────────────

// The empty-path -> $HOME default is the call every web/remote/TUI client makes
// when the New Agent dialog opens with no starting directory, and it carries a
// paragraph explaining why it must run BEFORE the guard (an empty path is
// otherwise unverifiable). Deleting it left the whole hub suite green and turned
// the directory picker into "path is outside the allowed workspace" for every
// headless client under the DEFAULT catalog delegation.
func TestListDirWithNoPathOpensTheHomeDirectory(t *testing.T) {
	home, _ := sandboxHome(t)
	reg := registryWithCwds(t)
	for _, params := range []string{`{"path":""}`, `{}`, `{"path":"   "}`} {
		res, err := reg.handle(context.Background(), "fs.listDir", json.RawMessage(params))
		if err != nil {
			t.Fatalf("fs.listDir %s: %v — the picker's opening call must be answered", params, err)
		}
		var out struct {
			Path string `json:"path"`
		}
		if err := json.Unmarshal(res, &out); err != nil {
			t.Fatal(err)
		}
		if out.Path != home {
			t.Errorf("fs.listDir %s listed %q, want the home directory %q", params, out.Path, home)
		}
	}
}

// ── fs.listEntries: a symlinked directory is a directory ─────────────────────

// fileService.ts does exactly this (`if (!isDir && e.isSymbolicLink()) isDir =
// statSync(full).isDirectory()`), and fs.listEntries is answered by whichever
// provider is registered. Deleting the Go half left the package green while a
// symlinked directory came back isDir:false from the brain and isDir:true from
// the desktop — and since the sort comparator keys on isDir first, the whole
// listing re-orders too.
func TestListEntriesResolvesASymlinkedDirectoryAsADirectory(t *testing.T) {
	sandboxHome(t)
	proj, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	real := filepath.Join(proj, "realdir")
	if err := os.MkdirAll(real, 0o755); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, real, filepath.Join(proj, "linkdir"))
	if err := os.WriteFile(filepath.Join(proj, "a.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(proj, "a.txt"), filepath.Join(proj, "linkfile")); err != nil {
		t.Fatal(err)
	}

	reg := registryWithCwd(t, proj)
	raw, _ := json.Marshal(map[string]string{"path": proj})
	res, err := reg.handle(context.Background(), "fs.listEntries", raw)
	if err != nil {
		t.Fatalf("fs.listEntries: %v", err)
	}
	var out listEntriesResult
	if err := json.Unmarshal(res, &out); err != nil {
		t.Fatal(err)
	}
	got := map[string]bool{}
	for _, e := range out.Entries {
		got[e.Name] = e.IsDir
	}
	if !got["linkdir"] {
		t.Error("a symlink to a DIRECTORY came back isDir:false — the desktop twin answers true, and the sort keys on isDir first, so the same call renders a different tree per provider")
	}
	if got["linkfile"] {
		t.Error("a symlink to a FILE came back isDir:true")
	}
	if !got["realdir"] {
		t.Error("floor: a real directory must still be isDir")
	}
	// The two entries the listing drops in both providers.
	for _, name := range []string{".git"} {
		if _, ok := got[name]; ok {
			t.Errorf("%s should not be listed", name)
		}
	}
}

// ── quarantineUnreadable's mode ──────────────────────────────────────────────

// 0o600 carries its own justifying comment — "a byte-for-byte copy of a file
// whose own mode we did not inspect, minted by a read-only capability" — and
// widening it to 0o644 killed nothing. The .broken-* copy is created inside
// <configDir>/layouts or <configDir>/sessions by layouts.list / sessions.list,
// which any remote bus caller can drive.
func TestQuarantineCopyIsNotWorldReadable(t *testing.T) {
	if onWindows {
		t.Skip("POSIX mode bits")
	}
	sandboxHome(t)
	dir := sessionsDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	bad := filepath.Join(dir, "broken.yaml")
	if err := os.WriteFile(bad, []byte("\tnot: [valid\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	quarantineUnreadable(bad, []byte("secret bytes"))

	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	found := 0
	for _, e := range entries {
		if !strings.Contains(e.Name(), ".broken-") {
			continue
		}
		found++
		info, err := e.Info()
		if err != nil {
			t.Fatal(err)
		}
		if perm := info.Mode().Perm(); perm != 0o600 {
			t.Errorf("%s has mode %#o, want 0600 — this is a copy of a file whose own mode was never inspected, minted by a read-only capability a remote caller drives", e.Name(), perm)
		}
	}
	if found != 1 {
		t.Fatalf("expected exactly one .broken-* copy, found %d", found)
	}
}

// ── BINDING DECISION 2 on the library WRITE legs ─────────────────────────────

// saveLibrary and saveLibraryClaude both compose a derived destination, guard it,
// and then `full = canonical` before writing. Dropping that one assignment on
// either leg survived the whole package: for an in-root symlink the bytes land in
// the same file either way, so the two are equivalent for a READ — but
// writeFileAtomic renames a temp file into place, which REPLACES a symlink with a
// regular file instead of writing through it. The desktop twin
// (libraryService.guardWriteTarget) is held to the same property.
func TestLibrarySaveWritesThroughTheGuardsAnswer(t *testing.T) {
	sandboxHome(t)
	proj, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	reg := registryWithCwd(t, proj)

	t.Run("project scope", func(t *testing.T) {
		dir := libraryProjectDir(proj)
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(dir, "target.md")
		link := filepath.Join(dir, "note.md")
		if err := os.WriteFile(target, []byte("old"), 0o644); err != nil {
			t.Fatal(err)
		}
		gateSymlink(t, target, link)
		raw, _ := json.Marshal(map[string]any{
			"scope": "project", "id": "note", "title": "Note", "kind": "prompt",
			"body": "NEWBODY", "cwd": proj,
		})
		if _, err := reg.handle(context.Background(), "library.save", raw); err != nil {
			t.Fatalf("library.save: %v", err)
		}
		st, err := os.Lstat(link)
		if err != nil {
			t.Fatal(err)
		}
		if st.Mode()&os.ModeSymlink == 0 {
			t.Error("the write REPLACED the in-store link with a regular file instead of writing the file the guard resolved to")
		}
		body, err := os.ReadFile(target)
		if err != nil || !strings.Contains(string(body), "NEWBODY") {
			t.Errorf("the resolved target was not written: %v / %q", err, body)
		}
	})

	t.Run("claude scope", func(t *testing.T) {
		dir := filepath.Join(claudeAgentsDir(proj))
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(proj, ".claude", "realagent.md")
		link := filepath.Join(dir, "reviewer.md")
		if err := os.WriteFile(target, []byte("old"), 0o644); err != nil {
			t.Fatal(err)
		}
		gateSymlink(t, target, link)
		raw, _ := json.Marshal(map[string]any{
			"scope": "claude", "kind": "agent", "id": "reviewer", "title": "reviewer",
			"body": "CLAUDEBODY", "cwd": proj,
		})
		if _, err := reg.handle(context.Background(), "library.save", raw); err != nil {
			t.Fatalf("library.save: %v", err)
		}
		st, err := os.Lstat(link)
		if err != nil {
			t.Fatal(err)
		}
		if st.Mode()&os.ModeSymlink == 0 {
			t.Error("the claude-scope write REPLACED the link instead of writing the resolved file")
		}
		body, err := os.ReadFile(target)
		if err != nil || !strings.Contains(string(body), "CLAUDEBODY") {
			t.Errorf("the resolved target was not written: %v / %q", err, body)
		}
	})
}
