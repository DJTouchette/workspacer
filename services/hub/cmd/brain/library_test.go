package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

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

	items := listLibrary("")
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

	items := listLibrary(cwd)
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
	for _, it := range listLibrary(cwd) {
		switch it.Kind {
		case "skill":
			s := it
			skill = &s
		case "agent":
			a := it
			agent = &a
		}
	}
	if skill == nil || skill.Scope != "claude" || skill.Title != "My Skill" || skill.ID != "myskill" {
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
	for _, it := range listLibrary(cwd) {
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
	if _, err := saveLibrary(libraryInput{Scope: "claude", Kind: "command", ID: "release", Title: "release", Body: "cut a release", Cwd: cwd}); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(claudeCommandsDir(cwd), "release.md")); err != nil {
		t.Fatalf("saved command not at .claude/commands/release.md: %v", err)
	}
	// …and remove deletes the command file, not a same-named skill dir.
	removeLibrary("claude", "deploy", cwd, "command")
	if _, err := os.Stat(filepath.Join(claudeCommandsDir(cwd), "deploy.md")); !os.IsNotExist(err) {
		t.Fatal("command file should be removed")
	}
}

func TestLibrarySaveAndRemoveGlobal(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())

	it, err := saveLibrary(libraryInput{Scope: "global", Title: "My Prompt", Kind: "prompt", Body: "hello {{x}}"})
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

	removeLibrary("global", "my-prompt", "", "")
	if _, err := os.Stat(filepath.Join(libraryGlobalDir(), "my-prompt.md")); !os.IsNotExist(err) {
		t.Fatal("file should be removed")
	}
}

func TestLibrarySaveClaudePreservesUnmodeledFrontmatter(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	cwd := t.TempDir()
	skillFile := filepath.Join(claudeSkillsDir(cwd), "foo", "SKILL.md")
	writeFile(t, skillFile, "---\nname: Old\ndescription: old\ntools:\n  - Read\nmodel: opus\n---\n\nold body\n")

	if _, err := saveLibrary(libraryInput{Scope: "claude", Kind: "skill", ID: "foo", Title: "New Title", Description: "new", Body: "new body", Cwd: cwd}); err != nil {
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
