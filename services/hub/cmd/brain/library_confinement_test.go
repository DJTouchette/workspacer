package main

// Derived-path confinement for library.* — the legs the earlier regression tests
// left free.
//
// listLibrary reaches the filesystem through FIVE walkers (global store, project
// store, .claude/skills, .claude/agents, .claude/commands) and the previous test
// planted a symlink in exactly two of them, so three per-file guards — including
// the GLOBAL one, whose own comment calls <configDir>/library "the shortest
// version of the same attack" because it is the one directory a remote caller can
// fs.write into — could be swapped for the identity guard with the whole Go suite
// green. Same story on the write side: library.save's derived guard was pinned on
// the non-claude branch only, and library.remove's derived ROOT SET (item roots,
// not workspace roots) was pinned by nothing at all even though its sink is
// os.RemoveAll.
//
// Everything here is table-driven over the legs rather than over one plant, so a
// sixth walker cannot be added without an entry.

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const libSecret = "SUPERSECRET-REMOTE-TOKEN"

// libraryLeg is one place listLibrary opens a file: where to plant the symlink,
// and where to put a legitimate item so the leg's floor is asserted too.
type libraryLeg struct {
	name string
	// plant is the path (relative to cwd, or "" for the global store) whose
	// parent directory is created and where the poisoned symlink goes.
	plant func(cwd string) string
	// real is a legitimate item in the same leg, and the title it must list under.
	real  func(cwd string) string
	title string
}

func libraryLegs() []libraryLeg {
	return []libraryLeg{
		{
			name:  "global store",
			plant: func(string) string { return filepath.Join(libraryGlobalDir(), "pwn.md") },
			real:  func(string) string { return filepath.Join(libraryGlobalDir(), "ok.md") },
			title: "GlobalFloor",
		},
		{
			name:  "project store",
			plant: func(cwd string) string { return filepath.Join(cwd, ".workspacer", "library", "pwn.md") },
			real:  func(cwd string) string { return filepath.Join(cwd, ".workspacer", "library", "ok.md") },
			title: "ProjectFloor",
		},
		{
			name:  "claude skills",
			plant: func(cwd string) string { return filepath.Join(cwd, ".claude", "skills", "pwn", "SKILL.md") },
			real:  func(cwd string) string { return filepath.Join(cwd, ".claude", "skills", "okskill", "SKILL.md") },
			title: "okskill",
		},
		{
			name:  "claude agents",
			plant: func(cwd string) string { return filepath.Join(cwd, ".claude", "agents", "pwn.md") },
			real:  func(cwd string) string { return filepath.Join(cwd, ".claude", "agents", "okagent.md") },
			title: "okagent",
		},
		{
			name:  "claude commands",
			plant: func(cwd string) string { return filepath.Join(cwd, ".claude", "commands", "pwn.md") },
			real:  func(cwd string) string { return filepath.Join(cwd, ".claude", "commands", "okcmd.md") },
			title: "okcmd",
		},
	}
}

func TestEveryLibraryListWalkerGuardsTheFileItOpens(t *testing.T) {
	for _, leg := range libraryLegs() {
		t.Run(leg.name, func(t *testing.T) {
			cwd, token := libraryCwdWithConfigDir(t)

			plant := leg.plant(cwd)
			if err := os.MkdirAll(filepath.Dir(plant), 0o755); err != nil {
				t.Fatal(err)
			}
			gateSymlink(t, token, plant)
			// The floor for this same leg, so a guard that simply refuses
			// everything cannot satisfy the assertion below.
			real := leg.real(cwd)
			if err := os.MkdirAll(filepath.Dir(real), 0o755); err != nil {
				t.Fatal(err)
			}
			body := "---\ntitle: " + leg.title + "\nname: " + leg.title + "\n---\n\nbody\n"
			if err := os.WriteFile(real, []byte(body), 0o644); err != nil {
				t.Fatal(err)
			}

			// Control: fs.read of the identical symlink is refused.
			reg := registryWithCwd(t, cwd)
			if _, err := reg.handle(context.Background(), "fs.read",
				json.RawMessage(`{"path":`+jsonStr(plant)+`}`)); err == nil {
				t.Fatal("fs.read of the planted symlink must be denied (the control for this test)")
			}

			reg = registryWithCwd(t, cwd)
			res, err := reg.handle(context.Background(), "library.list",
				json.RawMessage(`{"cwd":`+jsonStr(cwd)+`}`))
			if err != nil {
				t.Fatalf("library.list of a legitimate cwd must still succeed: %v", err)
			}
			if strings.Contains(string(res), libSecret) {
				t.Fatalf("the %s walker returned the bus credential through a planted symlink: %s", leg.name, res)
			}
			if !strings.Contains(string(res), leg.title) {
				t.Fatalf("the %s walker stopped listing its ordinary item %q — the guard must skip one file, not the leg: %s",
					leg.name, leg.title, res)
			}
		})
	}
}

// library.list's cwd is checked against the BROWSE roots (the whole home tree),
// so a caller may name $HOME itself — and then the derived-file allow-list
// [<configDir>/library, cwd] is the whole home tree again and narrows nothing.
// The shipped regression test only ever named $HOME/scratch, one component
// deeper, which is the one spelling an attacker does not have to use.
func TestLibraryListWithHomeAsTheCwdIsNotAnArbitraryHomeReader(t *testing.T) {
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	home := filepath.Join(sandbox, "home")
	if err := os.MkdirAll(filepath.Join(home, ".ssh"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	key := filepath.Join(home, ".ssh", "id_rsa")
	if err := os.WriteFile(key, []byte("-----BEGIN OPENSSH PRIVATE KEY-----\nSTOLEN\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	lib := filepath.Join(home, ".workspacer", "library")
	if err := os.MkdirAll(lib, 0o755); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, key, filepath.Join(lib, "a.md"))

	// Control: fs.read of the same path is refused (no live agents at all).
	reg := newRegistry(nil)
	if _, err := reg.handle(context.Background(), "fs.read",
		json.RawMessage(`{"path":`+jsonStr(filepath.Join(lib, "a.md"))+`}`)); err == nil {
		t.Fatal("fs.read of the planted symlink must be denied (the control for this test)")
	}

	for _, cwd := range []string{home, filepath.Join(home, "scratch")} {
		_ = os.MkdirAll(cwd, 0o755)
		reg := newRegistry(nil)
		res, err := reg.handle(context.Background(), "library.list",
			json.RawMessage(`{"cwd":`+jsonStr(cwd)+`}`))
		if err != nil {
			continue // a refusal is a perfectly good answer
		}
		if strings.Contains(string(res), "STOLEN") {
			t.Fatalf("library.list(cwd=%q) returned a home-directory file fs.read refuses: %s", cwd, res)
		}
	}
}

// The derived ROOT SET for the two mutating legs. Both compose their destination
// out of the caller's cwd and must confine it to the ITEM roots — [global store,
// that cwd] — not to the workspace roots, which include EVERY live agent cwd and
// all three config stores. With the wider set, one `.claude/skills -> <projB>`
// or `.workspacer/library -> <projB>` link inside the (legitimately allowed)
// project A writes into, and os.RemoveAll's out of, a second agent's tree.
func TestLibraryWritesAndDeletesStayInTheProjectTheCallerNamed(t *testing.T) {
	for _, tc := range []struct {
		name string
		// link is the directory inside projA replaced by a symlink to projB.
		link string
		// params for the mutating call.
		save   string
		remove string
		// victim is the path under projB the call would touch.
		victim string
	}{
		{
			name:   "claude scope",
			link:   filepath.Join(".claude", "skills"),
			save:   `{"scope":"claude","kind":"skill","id":"pwn","title":"t","body":"OWNED","cwd":%CWD%}`,
			remove: `{"scope":"claude","kind":"skill","id":"keep","cwd":%CWD%}`,
			victim: "pwn",
		},
		{
			name:   "project scope",
			link:   filepath.Join(".workspacer", "library"),
			save:   `{"scope":"project","id":"pwn","title":"t","body":"OWNED","cwd":%CWD%}`,
			remove: `{"scope":"project","id":"keep","cwd":%CWD%}`,
			victim: "pwn.md",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			sandbox, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
			t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
			projA := filepath.Join(sandbox, "projA")
			projB := filepath.Join(sandbox, "projB")
			for _, d := range []string{filepath.Join(projA, filepath.Dir(tc.link)), projB, configDir()} {
				if err := os.MkdirAll(d, 0o755); err != nil {
					t.Fatal(err)
				}
			}
			gateSymlink(t, projB, filepath.Join(projA, tc.link))
			// A file in projB the delete leg would destroy.
			keep := filepath.Join(projB, "keep")
			if tc.name == "project scope" {
				keep += ".md"
			}
			if err := os.MkdirAll(keep, 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(keep, "precious.txt"), []byte("precious"), 0o644); err != nil {
				t.Fatal(err)
			}

			// BOTH projects are live agent cwds, so the workspace roots contain
			// projB and only the narrower item roots can refuse.
			sub := func(s string) json.RawMessage {
				return json.RawMessage(strings.ReplaceAll(s, "%CWD%", jsonStr(projA)))
			}
			reg := registryWithCwds(t, projA, projB)
			if _, err := reg.handle(context.Background(), "library.save", sub(tc.save)); err == nil {
				t.Errorf("library.save through a symlinked item directory must be refused")
			}
			if _, err := os.Stat(filepath.Join(projB, tc.victim)); err == nil {
				t.Errorf("library.save landed attacker content in a SECOND agent's project: %s",
					filepath.Join(projB, tc.victim))
			}

			reg = registryWithCwds(t, projA, projB)
			if _, err := reg.handle(context.Background(), "library.remove", sub(tc.remove)); err != nil {
				t.Fatalf("library.remove reported an error rather than silently skipping: %v", err)
			}
			if _, err := os.Stat(filepath.Join(keep, "precious.txt")); err != nil {
				t.Errorf("library.remove destroyed a SECOND agent's tree through the symlink: %v", err)
			}
		})
	}
}

// The floor for the test above: the same two calls against an ordinary project
// must still write and still delete, or "refuse everything" would satisfy it.
func TestLibraryWritesAndDeletesStillWorkInAnOrdinaryProject(t *testing.T) {
	for _, tc := range []struct {
		name   string
		save   string
		remove string
		path   func(cwd string) string
	}{
		{
			name:   "claude scope",
			save:   `{"scope":"claude","kind":"skill","id":"mine","title":"t","body":"hi","cwd":%CWD%}`,
			remove: `{"scope":"claude","kind":"skill","id":"mine","cwd":%CWD%}`,
			path:   func(cwd string) string { return filepath.Join(cwd, ".claude", "skills", "mine", "SKILL.md") },
		},
		{
			name:   "project scope",
			save:   `{"scope":"project","id":"mine","title":"t","body":"hi","cwd":%CWD%}`,
			remove: `{"scope":"project","id":"mine","cwd":%CWD%}`,
			path:   func(cwd string) string { return filepath.Join(cwd, ".workspacer", "library", "mine.md") },
		},
		{
			name:   "global scope",
			save:   `{"scope":"global","id":"mine","title":"t","body":"hi","cwd":%CWD%}`,
			remove: `{"scope":"global","id":"mine","cwd":%CWD%}`,
			path:   func(string) string { return filepath.Join(libraryGlobalDir(), "mine.md") },
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			sandbox, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
			t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
			cwd := filepath.Join(sandbox, "proj")
			if err := os.MkdirAll(cwd, 0o755); err != nil {
				t.Fatal(err)
			}
			sub := func(s string) json.RawMessage {
				return json.RawMessage(strings.ReplaceAll(s, "%CWD%", jsonStr(cwd)))
			}
			reg := registryWithCwd(t, cwd)
			if _, err := reg.handle(context.Background(), "library.save", sub(tc.save)); err != nil {
				t.Fatalf("an ordinary library.save must succeed: %v", err)
			}
			if _, err := os.Stat(tc.path(cwd)); err != nil {
				t.Fatalf("library.save wrote nothing at %s: %v", tc.path(cwd), err)
			}
			reg = registryWithCwd(t, cwd)
			if _, err := reg.handle(context.Background(), "library.remove", sub(tc.remove)); err != nil {
				t.Fatal(err)
			}
			if _, err := os.Stat(tc.path(cwd)); err == nil {
				t.Fatalf("library.remove left %s standing", tc.path(cwd))
			}
		})
	}
}
