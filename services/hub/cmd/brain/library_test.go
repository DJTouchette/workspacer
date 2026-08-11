package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// firstNonEmpty backs the library title/name fallback and must trim the SAME
// whitespace set as the desktop twin (libraryService.ts hasNonBlankText). Go's
// strings.TrimSpace strips U+0085 (NEL) but not U+FEFF (BOM); JS `.trim()` does
// the opposite, so a title that is a lone NEL or BOM must resolve to the SAME
// string on both providers (or the library picker shows a different row label
// AND a different byteCompare sort slot depending on DELEGATE_CATALOG_TO_BRAIN).
// The NEL case is the killer: reverting to strings.TrimSpace drops it to the id.
func TestFirstNonEmptyTrimsASCIIWhitespaceOnly(t *testing.T) {
	cases := []struct {
		name, in, want string
	}{
		{"ascii spaces fall back to id", "   ", "id"},
		{"tab/newline/cr fall back to id", "\t\n\v\f\r ", "id"},
		{"NEL-only title is kept (TrimSpace would drop it)", "\u0085", "\u0085"},
		{"BOM-only title is kept", "\ufeff", "\ufeff"},
		{"real title is kept", "Notes", "Notes"},
	}
	for _, c := range cases {
		if got := firstNonEmpty(c.in, "id"); got != c.want {
			t.Errorf("%s: firstNonEmpty(%q, \"id\") = %q, want %q", c.name, c.in, got, c.want)
		}
	}
}

func TestSlugLibrary(t *testing.T) {
	cases := map[string]string{
		"My Prompt!!":  "my-prompt",
		"  a  b  ":     "a-b",
		"keep_under":   "keep_under",
		"###":          "item",         // empty → fallback
		"Trim--Dashes": "trim--dashes", // library variant keeps internal dashes (no dedup)
	}
	for in, want := range cases {
		if got := slugLibrary(in); got != want {
			t.Errorf("slugLibrary(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestLibrarySeedAndList(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())

	items := listLibrary("", allowAnyLibraryFile)
	if len(items) != 4 {
		t.Fatalf("expected 4 seeded items, got %d", len(items))
	}
	// Sorted by title: "Careful refactor…", "Context7 (MCP)", "Make a workspacer
	// plugin…", "Summarize & plan".
	if items[0].Title != "Careful refactor (skill)" {
		t.Errorf("not sorted by title: %q first", items[0].Title)
	}
	var mcp *libraryItem
	for i := range items {
		if items[i].Kind == "mcp" {
			mcp = &items[i]
		}
	}
	if mcp == nil || mcp.Mcp == nil || mcp.Mcp.Command != "npx" || len(mcp.Mcp.Args) != 2 {
		t.Fatalf("mcp item didn't round-trip: %+v", mcp)
	}
}

func TestLibraryProjectOverridesGlobal(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	cwd := t.TempDir()

	// A global item suppresses seeding and gives us a known id.
	writeFile(t, filepath.Join(libraryGlobalDir(), "foo.md"), "---\ntitle: Global Foo\n---\n\nglobal body\n")
	writeFile(t, filepath.Join(libraryProjectDir(cwd), "foo.md"), "---\ntitle: Project Foo\n---\n\nproject body\n")

	items := listLibrary(cwd, allowAnyLibraryFile)
	var foo *libraryItem
	for i := range items {
		if items[i].ID == "foo" {
			foo = &items[i]
		}
	}
	if foo == nil || foo.Scope != "project" || foo.Title != "Project Foo" {
		t.Fatalf("project should win on id collision, got %+v", foo)
	}
}

func TestLibraryClaudeAssets(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	cwd := t.TempDir()
	writeFile(t, filepath.Join(libraryGlobalDir(), "x.md"), "---\ntitle: X\n---\n\nx\n") // suppress seed
	writeFile(t, filepath.Join(claudeSkillsDir(cwd), "MySkill", "SKILL.md"), "---\nname: My Skill\ndescription: d\n---\n\nskill body\n")
	writeFile(t, filepath.Join(claudeAgentsDir(cwd), "myAgent.md"), "---\nname: My Agent\n---\n\nagent body\n")

	var skill, agent *libraryItem
	for _, it := range listLibrary(cwd, allowAnyLibraryFile) {
		switch it.Kind {
		case "skill":
			s := it
			skill = &s
		case "agent":
			a := it
			agent = &a
		}
	}
	// "MySkill", not "myskill": a claude-scoped id is the item's REAL on-disk
	// basename, so save/remove can address the directory that is actually there.
	// This assertion used to demand the slug, which is what let the two providers
	// disagree — libraryService.ts has never slugged a claude id.
	if skill == nil || skill.Scope != "claude" || skill.Title != "My Skill" || skill.ID != "MySkill" {
		t.Fatalf("skill not discovered correctly: %+v", skill)
	}
	if agent == nil || agent.Title != "My Agent" {
		t.Fatalf("agent not discovered correctly: %+v", agent)
	}
}

func TestLibraryClaudeCommands(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	cwd := t.TempDir()
	writeFile(t, filepath.Join(libraryGlobalDir(), "x.md"), "---\ntitle: X\n---\n\nx\n") // suppress seed
	// Claude command frontmatter carries no `name`; the filename is the command.
	writeFile(t, filepath.Join(claudeCommandsDir(cwd), "deploy.md"), "---\ndescription: Ship it\n---\n\nRun the deploy playbook.\n")

	var cmd *libraryItem
	for _, it := range listLibrary(cwd, allowAnyLibraryFile) {
		if it.Kind == "command" {
			c := it
			cmd = &c
		}
	}
	if cmd == nil || cmd.Scope != "claude" || cmd.ID != "deploy" {
		t.Fatalf("command not discovered: %+v", cmd)
	}
	// No `name` in frontmatter → title falls back to the id (the "/deploy" name).
	if cmd.Title != "deploy" || cmd.Description != "Ship it" {
		t.Fatalf("command title/description wrong: %+v", cmd)
	}

	// Save routes a command to .claude/commands (not skills/agents)…
	reg := registryWithCwd(t, cwd)
	if _, err := reg.saveLibrary(context.Background(), libraryInput{Scope: "claude", Kind: "command", ID: "release", Title: "release", Body: "cut a release", Cwd: cwd}); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(claudeCommandsDir(cwd), "release.md")); err != nil {
		t.Fatalf("saved command not at .claude/commands/release.md: %v", err)
	}
	// …and remove deletes the command file, not a same-named skill dir.
	removeLibrary("claude", "deploy", cwd, "command", allowAnyLibraryFile)
	if _, err := os.Stat(filepath.Join(claudeCommandsDir(cwd), "deploy.md")); !os.IsNotExist(err) {
		t.Fatal("command file should be removed")
	}
}

func TestLibrarySaveAndRemoveGlobal(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())

	reg := registryWithCwd(t, t.TempDir())
	it, err := reg.saveLibrary(context.Background(), libraryInput{Scope: "global", Title: "My Prompt", Kind: "prompt", Body: "hello {{x}}"})
	if err != nil {
		t.Fatal(err)
	}
	if it.ID != "my-prompt" {
		t.Fatalf("id = %q, want my-prompt", it.ID)
	}
	raw := readFile(t, filepath.Join(libraryGlobalDir(), "my-prompt.md"))
	if !strings.Contains(raw, "title: My Prompt") || !strings.Contains(raw, "kind: prompt") || !strings.Contains(raw, "hello {{x}}") {
		t.Fatalf("serialized file missing expected content:\n%s", raw)
	}

	removeLibrary("global", "my-prompt", "", "", allowAnyLibraryFile)
	if _, err := os.Stat(filepath.Join(libraryGlobalDir(), "my-prompt.md")); !os.IsNotExist(err) {
		t.Fatal("file should be removed")
	}
}

// removeLibrary's claude kind="agent" leg. The coverage profile reported both of
// its lines as never executed: the three kinds share one `remove` closure, so the
// skill and command legs looked like they covered it, and neither of the two
// things that make this leg DIFFERENT from its siblings — which directory it
// points at, and that it is the non-recursive form — was pinned.
//
// library.remove is a destructive bus capability whose surrounding comments are
// entirely about deleting the wrong item ("remove(id=\"My.Skill\") unlinked
// my-skill and left My.Skill standing"), so both halves get an assertion:
//
//	claudeAgentsDir -> claudeCommandsDir  deletes the slash command of the same
//	                                      name and leaves the agent standing;
//	false -> true                         turns one unlink into os.RemoveAll,
//	                                      which is how the skill leg erases a
//	                                      whole directory tree.
func TestClaudeAgentRemoveUnlinksOneFileInTheAgentsDirectory(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	cwd := t.TempDir()
	agent := filepath.Join(claudeAgentsDir(cwd), "reviewer.md")
	command := filepath.Join(claudeCommandsDir(cwd), "reviewer.md")
	writeFile(t, agent, "---\nname: Reviewer\n---\n\nbody\n")
	writeFile(t, command, "---\n---\n\nslash command\n")

	reg := registryWithCwd(t, cwd)
	if _, err := reg.handle(context.Background(), "library.remove",
		json.RawMessage(`{"scope":"claude","kind":"agent","id":"reviewer","cwd":`+jsonStr(cwd)+`}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(agent); !os.IsNotExist(err) {
		t.Errorf("the agent must be removed: %v", err)
	}
	if _, err := os.Stat(command); err != nil {
		t.Errorf("the SLASH COMMAND of the same name must be left standing: %v", err)
	}

	// The recursive half. A directory sitting at the agent's path is unusual but
	// it is the only thing that separates os.Remove from os.RemoveAll, and the
	// separation is the contract: only the skill leg deletes a tree.
	tree := filepath.Join(claudeAgentsDir(cwd), "notes.md")
	writeFile(t, filepath.Join(tree, "inside.txt"), "keep me")
	if _, err := reg.handle(context.Background(), "library.remove",
		json.RawMessage(`{"scope":"claude","kind":"agent","id":"notes","cwd":`+jsonStr(cwd)+`}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(tree, "inside.txt")); err != nil {
		t.Errorf("the agent leg is a single unlink, not a recursive delete: %v", err)
	}
}

func TestLibrarySaveClaudePreservesUnmodeledFrontmatter(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	cwd := t.TempDir()
	skillFile := filepath.Join(claudeSkillsDir(cwd), "foo", "SKILL.md")
	writeFile(t, skillFile, "---\nname: Old\ndescription: old\ntools:\n  - Read\nmodel: opus\n---\n\nold body\n")

	reg := registryWithCwd(t, cwd)
	if _, err := reg.saveLibrary(context.Background(), libraryInput{Scope: "claude", Kind: "skill", ID: "foo", Title: "New Title", Description: "new", Body: "new body", Cwd: cwd}); err != nil {
		t.Fatal(err)
	}
	data, _ := parseFrontmatter(readFile(t, skillFile))
	if data["name"] != "New Title" {
		t.Errorf("name should update, got %v", data["name"])
	}
	if data["model"] != "opus" || data["tools"] == nil {
		t.Errorf("unmodeled keys (tools/model) must be preserved, got %+v", data)
	}
}

// TestLibrarySaveIsConfinedToTheWorkspace: library.save takes the cwd it writes
// under from the caller, so it has to answer to the same fsguard containment as
// fs.write. Without it the method is a second, unguarded write primitive sitting
// next to the guarded one — the exact drift that left the brain's fs.* handlers
// unconfined while the desktop twin looked fixed.
func TestLibrarySaveIsConfinedToTheWorkspace(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	agentCwd := t.TempDir()
	elsewhere := t.TempDir() // no agent runs here
	reg := registryWithCwd(t, agentCwd)

	if _, err := reg.saveLibrary(context.Background(), libraryInput{
		Scope: "project", Title: "Pwn", Kind: "prompt", Body: "x", Cwd: elsewhere,
	}); err == nil {
		t.Error("library.save into a directory with no live agent should be denied")
	}
	if _, err := os.Stat(libraryProjectDir(elsewhere)); !os.IsNotExist(err) {
		t.Errorf("a denied library.save must not create directories: %v", err)
	}

	// The claude scope writes through a different branch — and its own check.
	if _, err := reg.saveLibrary(context.Background(), libraryInput{
		Scope: "claude", Kind: "skill", ID: "pwn", Title: "Pwn", Body: "x", Cwd: elsewhere,
	}); err == nil {
		t.Error("library.save (claude scope) outside the workspace should be denied")
	}

	// …and the legitimate write into a live agent's project still lands.
	if _, err := reg.saveLibrary(context.Background(), libraryInput{
		Scope: "project", Title: "Ok", Kind: "prompt", Body: "x", Cwd: agentCwd,
	}); err != nil {
		t.Fatalf("library.save inside a live agent cwd should be allowed: %v", err)
	}
}

// helpers

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func readFile(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

// library.save has always been path-guarded; list and remove were not — and
// under the default DELEGATE_CATALOG_TO_BRAIN these are the copies that run, so
// the desktop's guarded twin never sees the call.
func TestLibraryListAndRemoveRejectAnEscapingCwd(t *testing.T) {
	dir := t.TempDir()
	reg := registryWithCwd(t, dir)

	// Inside the agent cwd is fine.
	if _, err := reg.handle(context.Background(), "library.list",
		json.RawMessage(`{"cwd":`+jsonStr(dir)+`}`)); err != nil {
		t.Fatalf("list inside the agent cwd should be allowed: %v", err)
	}

	// Outside every workspace root is not.
	for _, method := range []string{"library.list", "library.remove"} {
		if _, err := reg.handle(context.Background(), method,
			json.RawMessage(`{"cwd":"/etc","scope":"project","id":"x","kind":"agent"}`)); err == nil {
			t.Errorf("%s accepted a cwd outside every workspace root", method)
		}
	}
}

// library.list checks its CWD against the browse roots — workspace roots plus
// the whole home tree — because the New Agent dialog lists the library of a
// directory no agent is running in yet. Handing those same roots to the
// PER-FILE guard is a different thing entirely: fs.listDir browses the home tree
// and returns directory NAMES, while this call returns file BODIES. With the
// browse roots on the files, one symlink anywhere under $HOME (git stores
// symlink values verbatim, so a clone carries them) turned library.list into an
// arbitrary home-directory reader — ~/.ssh/id_rsa and ~/.claude/.credentials.json
// came back as an item Body, while fs.read of the identical path is refused.
//
// A library item lives in the project the caller named or in the global store.
// Nowhere else, whatever the cwd was allowed against.
func TestLibraryListDoesNotReadOutsideTheProjectItNamed(t *testing.T) {
	home := tempHome(t)
	t.Setenv("USERPROFILE", home)
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())

	const secret = "-----BEGIN OPENSSH PRIVATE KEY-----\nSTOLEN\n"
	if err := os.MkdirAll(filepath.Join(home, ".ssh"), 0o700); err != nil {
		t.Fatal(err)
	}
	key := filepath.Join(home, ".ssh", "id_rsa")
	if err := os.WriteFile(key, []byte(secret), 0o600); err != nil {
		t.Fatal(err)
	}

	project := filepath.Join(home, "scratch")
	libDir := filepath.Join(project, ".workspacer", "library")
	if err := os.MkdirAll(libDir, 0o755); err != nil {
		t.Fatal(err)
	}
	// The floor, alongside the plant: a genuine item in the project must still
	// be listed, or a guard that refused everything would pass this test.
	if err := os.WriteFile(filepath.Join(libDir, "keeper.md"),
		[]byte("---\ntitle: Keeper\nkind: prompt\n---\n\nbody\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, key, filepath.Join(libDir, "a.md"))

	// No live agents at all: `project` is reachable only through the BROWSE
	// widening, which is exactly the configuration the New Agent dialog is in.
	reg := registryWithCwds(t)
	raw, err := reg.handle(context.Background(), "library.list",
		json.RawMessage(`{"cwd":`+jsonStr(project)+`}`))
	if err != nil {
		t.Fatalf("library.list of a browsable directory should be allowed: %v", err)
	}
	var items []libraryItem
	if err := json.Unmarshal(raw, &items); err != nil {
		t.Fatal(err)
	}

	titles := []string{}
	for _, it := range items {
		titles = append(titles, it.Title)
		if strings.Contains(it.Body, "STOLEN") || strings.Contains(it.Path, ".ssh") {
			t.Fatalf("library.list returned a file outside the project it was given: id=%q path=%q", it.ID, it.Path)
		}
	}
	found := false
	for _, title := range titles {
		if title == "Keeper" {
			found = true
		}
	}
	if !found {
		t.Fatalf("the project's own item disappeared; got %v", titles)
	}

	// The control: fs.read of the identical path is refused, which is the
	// disagreement this test exists to close.
	if _, err := reg.handle(context.Background(), "fs.read",
		json.RawMessage(`{"path":`+jsonStr(filepath.Join(libDir, "a.md"))+`}`)); err == nil {
		t.Fatal("fs.read of the planted symlink must be denied (control)")
	}
}
