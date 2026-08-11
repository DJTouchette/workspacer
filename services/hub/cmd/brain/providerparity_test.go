package main

// Cross-provider parity for the three defects that were NOT in the containment
// predicate but were still one bus method answered two different ways.
//
// contracts/path-containment-cases.json pins the predicate and, through its
// `methods` block, that every path-bearing capability calls it. Neither reaches
// these: two providers can both guard correctly and still hand a caller
// different bytes, delete different files, or reach different directories. Each
// test below names the twin it is holding this side to.

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// ---------------------------------------------------------------------------
// fs.listEntries: the gitignore filter's wire protocol.

// TestListEntriesHidesGitignoredNamesTheDesktopHides is the Go half of
// fileService.gitignore.test.ts, which the port never got.
//
// `git check-ignore --stdin` echoes back the paths it matched, so the delimiter
// is a protocol both ends have to agree on. The desktop passes `-z` and
// NUL-delimits stdin and stdout because a filename may legally contain a
// NEWLINE: with a linefeed-delimited protocol git sees "a\nb.log" as the two
// paths "a" and "b.log", echoes "b.log", and the readdir name "a\nb.log" is
// never marked ignored — so the file is listed. Only half that fix crossed over:
// core.quotePath=false (the unicode half) made it, `-z` did not, and the brain's
// only listEntries test used a plain "ignored.txt" that both protocols get right.
//
// fs.listEntries has two providers (the corpus says so) and .gitignore is what
// keeps a project's deliberately-hidden files out of the editor tree the web,
// remote and PWA clients render over the bus. Disclosure, not an escape — the
// containment guard runs before either sink — but exactly the drift the corpus
// exists to catch.
func TestListEntriesHidesGitignoredNamesTheDesktopHides(t *testing.T) {
	gateGit(t)
	dir := t.TempDir()
	if err := exec.Command("git", "-C", dir, "init").Run(); err != nil {
		t.Skipf("git init failed: %v", err)
	}
	writeFile(t, filepath.Join(dir, ".gitignore"), "*.log\n")
	writeFile(t, filepath.Join(dir, "keep.txt"), "k")

	// The same three shapes the desktop's regression test builds. The newline
	// name is the one the delimiter decides; the other two are the control.
	hidden := []string{"ascii.log", "é.log", "a\nb.log"}
	for _, name := range hidden {
		if err := os.WriteFile(filepath.Join(dir, name), []byte("secret"), 0o644); err != nil {
			t.Skipf("this filesystem will not hold %q: %v", name, err)
		}
	}

	res, err := listEntries(dir)
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]bool{}
	for _, e := range res.Entries {
		got[e.Name] = true
	}
	for _, name := range hidden {
		if got[name] {
			t.Errorf("fs.listEntries returned the gitignored file %q — the desktop provider of the same method hides it (fileService.gitignore.test.ts). A linefeed-delimited check-ignore protocol splits a name containing a newline into two bogus paths, so the echoed match never equals the readdir name; pass -z and NUL-delimit both directions.", name)
		}
	}
	if !got["keep.txt"] || !got[".gitignore"] {
		t.Errorf("the filter swallowed files nothing ignores: %+v", res.Entries)
	}
}

// ---------------------------------------------------------------------------
// claude.sessionsForDir: the ~/.claude/projects slug.

type projectDirNameCase struct {
	Cwd    string  `json:"cwd"`
	Expect *string `json:"expect"`
	Why    string  `json:"why"`
}

// TestClaudeProjectDirNameContractCases loads the corpus's `projectDirNames`
// block — the invariant capspec's exemption for claude.sessionsForDir rests on.
// The desktop twin (claudeSessionList.ts) is held to the same block.
func TestClaudeProjectDirNameContractCases(t *testing.T) {
	raw := readContractFixtureBytes(t)
	var fx struct {
		ProjectDirNames struct {
			Cases []projectDirNameCase `json:"cases"`
		} `json:"projectDirNames"`
	}
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", contractFixtureRel, err)
	}
	if len(fx.ProjectDirNames.Cases) == 0 {
		t.Fatalf("%s decoded to zero projectDirNames cases — a silently empty block guards nothing", contractFixtureRel)
	}

	// projectDirNamesFloor: the block's size today. `len(...) == 0` above is met
	// by a block that lost seven of its eight cases.
	const projectDirNamesFloor = 8
	var tally sweepguard.Tally
	for _, c := range fx.ProjectDirNames.Cases {
		t.Run(c.Cwd, func(t *testing.T) {
			// Filed by verdict: a nil `expect` is a refusal, and a sweep that
			// ran only refusals is satisfied by an encoder that refuses
			// everything.
			if c.Expect == nil {
				tally.Ran("refuse")
			} else {
				tally.Ran("accept")
			}
			got, ok := claudeProjectDirName(c.Cwd)
			if c.Expect == nil {
				if ok {
					t.Fatalf("claudeProjectDirName(%q) produced %q; the corpus requires a refusal\n  why: %s", c.Cwd, got, c.Why)
				}
				return
			}
			if !ok {
				t.Fatalf("claudeProjectDirName(%q) refused; the corpus expects %q\n  why: %s", c.Cwd, *c.Expect, c.Why)
			}
			if got != *c.Expect {
				t.Fatalf("claudeProjectDirName(%q) = %q, want %q\n  why: %s", c.Cwd, got, *c.Expect, c.Why)
			}
			// Whatever it produces must be ONE plain component, or the exemption's
			// sentence ("never opened as a path") is still not true.
			if got == "" || got == "." || got == ".." || strings.ContainsAny(got, `/\`) {
				t.Fatalf("claudeProjectDirName(%q) = %q, which is not a single plain component", c.Cwd, got)
			}
		})
	}
	if err := tally.RequireEvery("the projectDirNames block", projectDirNamesFloor); err != nil {
		t.Fatal(err)
	}
	// Both classes: the refusals are what the capspec exemption rests on, and
	// the accepts are the only thing that says the encoder still encodes.
	if err := tally.Require("the projectDirNames block", 1, 1); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

// TestSessionsForDirCannotClimbOutOfTheProjectsDir is the behavioural half: the
// corpus case above pins the encoder, this drives the real handler and proves
// the handler consults it.
//
// Before the fix, cwd=".." made filepath.Join(home, ".claude", "projects", "..")
// Clean to ~/.claude, and the handler enumerated every *.jsonl there — on a real
// machine, ~/.claude/history.jsonl, the user's entire prompt history — returning
// its name, mtime and up to 100 characters of extracted content.
func TestSessionsForDirCannotClimbOutOfTheProjectsDir(t *testing.T) {
	home := tempHome(t)
	t.Setenv("USERPROFILE", home)
	// One level ABOVE the transcript sandbox, which is where ~/.claude's own
	// history.jsonl lives.
	writeFile(t, filepath.Join(home, ".claude", "LEAK.jsonl"),
		`{"type":"summary","summary":"CONTENTS OF ~/.claude/LEAK.jsonl"}`+"\n")
	// And one legitimate project, so the assertion is not satisfied by a
	// handler that returns nothing at all.
	writeFile(t, filepath.Join(home, ".claude", "projects", "-proj", "ok.jsonl"),
		`{"type":"summary","summary":"a real transcript"}`+"\n")

	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	call := func(cwd string) string {
		t.Helper()
		res, err := reg.handle(context.Background(), "claude.sessionsForDir",
			json.RawMessage(`{"cwd":`+jsonStr(cwd)+`}`))
		if err != nil {
			return ""
		}
		return string(res)
	}

	for _, cwd := range []string{"..", ".", "/proj/.."} {
		if out := call(cwd); strings.Contains(out, "LEAK") {
			t.Errorf("claude.sessionsForDir(cwd=%q) reached one level out of ~/.claude/projects and disclosed %s", cwd, out)
		}
	}
	if out := call("/proj"); !strings.Contains(out, "a real transcript") {
		t.Errorf("claude.sessionsForDir stopped resolving legitimate project dirs too: %s", out)
	}
}

// ---------------------------------------------------------------------------
// library.*: derivedRootSet, and claude ids that are basenames.

// TestLibraryDerivedRootSetIsTheItemRoots reads the corpus's `derivedRootSet`
// column and holds the brain to it.
//
// The `methods` block's rootSet pins which allow-list the caller's `cwd` is
// checked against; derivedRootSet pins the SECOND, narrower list the paths
// composed from that cwd are checked against — [<configDir>/library, cwd]. The
// brain used r.workspaceRoots(ctx) for the derived write, which contains every
// OTHER live agent's cwd and all three config stores, so a directory symlink at
// the derived location (an ordinary permitted fs.write, and the form a git clone
// carries verbatim) sent the write into a second project and into
// <configDir>/sessions — where the desktop, on the item roots, refused.
func TestLibraryDerivedRootSetIsTheItemRoots(t *testing.T) {
	fx := loadContractFixture(t)
	derived := map[string]string{}
	for _, m := range fx.Methods {
		if m.DerivedRootSet != "" {
			derived[m.Method] = m.DerivedRootSet
		}
	}
	for _, method := range []string{"library.list", "library.save", "library.remove"} {
		if derived[method] != "item" {
			t.Fatalf("the corpus must declare derivedRootSet=item for %s (got %q) — without it nothing says the derived paths get their own, narrower list", method, derived[method])
		}
	}

	cfg := tempConfigHome(t)
	projA, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	projB, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	// BOTH are live agent cwds, so both are inside the WORKSPACE roots. Only
	// projA is inside the item roots for a call naming projA.
	reg := registryWithCwds(t, projA, projB)

	if err := os.MkdirAll(filepath.Join(projA, ".workspacer"), 0o755); err != nil {
		t.Fatal(err)
	}
	for _, target := range []string{projB, filepath.Join(cfg, "workspacer", "sessions")} {
		link := filepath.Join(projA, ".workspacer", "library")
		_ = os.RemoveAll(link)
		if err := os.MkdirAll(target, 0o755); err != nil {
			t.Fatal(err)
		}
		gateSymlink(t, target, link)
		body := `{"scope":"project","cwd":` + jsonStr(projA) + `,"id":"pwn","title":"t","kind":"prompt","body":"OWNED"}`
		res, err := reg.handle(context.Background(), "library.save", json.RawMessage(body))
		if err == nil {
			t.Errorf("library.save with cwd=%s wrote through a symlink into %s and returned %s — the derived destination must be confined to the item roots, not the workspace roots", projA, target, res)
		}
		if data, err := os.ReadFile(filepath.Join(target, "pwn.md")); err == nil {
			t.Errorf("library.save landed bytes in %s: %q", target, data)
		}
	}

	// The floor: an ordinary save into the project the caller named still works,
	// and what save creates, remove can delete.
	_ = os.RemoveAll(filepath.Join(projA, ".workspacer", "library"))
	body := `{"scope":"project","cwd":` + jsonStr(projA) + `,"id":"notes","title":"t","kind":"prompt","body":"ok"}`
	if _, err := reg.handle(context.Background(), "library.save", json.RawMessage(body)); err != nil {
		t.Fatalf("an ordinary project save must still be allowed: %v", err)
	}
	item := filepath.Join(projA, ".workspacer", "library", "notes.md")
	if _, err := os.Lstat(item); err != nil {
		t.Fatalf("library.save reported success but wrote nothing to %s: %v", item, err)
	}
	if _, err := reg.handle(context.Background(), "library.remove",
		json.RawMessage(`{"scope":"project","id":"notes","cwd":`+jsonStr(projA)+`}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Lstat(item); err == nil {
		t.Error("save and remove are on different root sets again: the brain created an item its own remove will not delete")
	}
}

// TestClaudeScopeIdsAreRealBasenames holds this side to the rule
// libraryService.ts states in a comment and enforces in code: a claude-scoped id
// is the item's REAL on-disk basename, never a slug of it.
//
// The brain slugged, in all three places (readClaudeItems, saveLibraryClaude,
// removeLibrary), so a project holding both `My.Skill` and `my-skill` listed ONE
// item where the desktop listed two, and library.remove(id="My.Skill") deleted
// `my-skill` — a skill the caller never named — while leaving `My.Skill` on
// disk. The trigger is any uppercase letter, dot or space in a skill name, not
// an exotic codepoint.
func TestClaudeScopeIdsAreRealBasenames(t *testing.T) {
	tempConfigHome(t)
	cwd, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(libraryGlobalDir(), "x.md"), "---\ntitle: X\n---\n\nx\n") // suppress the seed
	writeFile(t, filepath.Join(claudeSkillsDir(cwd), "My.Skill", "SKILL.md"), "---\nname: Mine\n---\n\nbody\n")
	writeFile(t, filepath.Join(claudeSkillsDir(cwd), "my-skill", "SKILL.md"), "---\nname: Other\n---\n\nbody\n")

	ids := func() []string {
		out := []string{}
		for _, it := range listLibrary(cwd, allowAnyLibraryFile) {
			if it.Scope == "claude" {
				out = append(out, it.ID)
			}
		}
		sort.Strings(out)
		return out
	}
	if got := ids(); len(got) != 2 || got[0] != "My.Skill" || got[1] != "my-skill" {
		t.Fatalf("library.list returned claude ids %v, want [My.Skill my-skill] — slugging collapses two real directories onto one map key and one of them silently disappears", got)
	}

	reg := registryWithCwds(t, cwd)
	if _, err := reg.handle(context.Background(), "library.remove",
		json.RawMessage(`{"scope":"claude","kind":"skill","id":"My.Skill","cwd":`+jsonStr(cwd)+`}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Lstat(filepath.Join(claudeSkillsDir(cwd), "My.Skill")); err == nil {
		t.Error(`library.remove(id="My.Skill") left My.Skill on disk`)
	}
	if _, err := os.Lstat(filepath.Join(claudeSkillsDir(cwd), "my-skill")); err != nil {
		t.Error(`library.remove(id="My.Skill") destroyed my-skill, an item the caller did not name`)
	}

	// save addresses the same real basename, so an edit lands in place.
	res, err := reg.handle(context.Background(), "library.save",
		json.RawMessage(`{"scope":"claude","kind":"skill","id":"my-skill","title":"T","body":"B","cwd":`+jsonStr(cwd)+`}`))
	if err != nil {
		t.Fatal(err)
	}
	var saved libraryItem
	if err := json.Unmarshal(res, &saved); err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(claudeSkillsDir(cwd), "my-skill", "SKILL.md"); saved.Path != want {
		t.Errorf("library.save wrote %q, want %q", saved.Path, want)
	}

	// And a supplied id still has to look like a basename: it reaches
	// filepath.Join unfiltered precisely because it is not slugged.
	for _, bad := range []string{"../../..", "a/b", ".."} {
		if _, err := reg.handle(context.Background(), "library.save",
			json.RawMessage(`{"scope":"claude","kind":"skill","id":`+jsonStr(bad)+`,"title":"T","body":"B","cwd":`+jsonStr(cwd)+`}`)); err == nil {
			t.Errorf("library.save accepted the claude id %q — an unslugged id is caller data reaching filepath.Join", bad)
		}
	}
}
