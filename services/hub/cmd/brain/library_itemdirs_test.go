package main

// The ITEM-DIRECTORY half of the library derived-path gate.
//
// assertLibraryItemPath runs two checks: assertPathAllowed over libraryItemRoots
// (pinned by library_confinement_test.go) and then `containsPath(dir, canonical)`
// over libraryItemDirs. Only the first half had coverage. The second is a second,
// independent copy of the containment comparison — the corpus pins the shape of
// the ROOTS comparison with four prefix-collision cases and an argument-order
// mutation, and this one had neither — plus a load-bearing decision that lives
// only in a comment ("Lexical on purpose. Canonicalizing these would resolve a
// `<cwd>/.workspacer/library -> <projB>` link and hand the escape back").
//
// Each case below is the shape of one surviving mutation of those two lines:
//
//	libraryItemDirs canonicalizes the project dirs   → library.list becomes an
//	                                                   arbitrary $HOME reader again
//	containsPath    → strings.HasPrefix              → <cwd>/.claude.json (the OAuth
//	                                                   blob) is "inside" <cwd>/.claude
//	containsPath(dir, c) → containsPath(c, dir)      → every ANCESTOR of an item dir
//	                                                   passes, and library.remove
//	                                                   os.RemoveAll's the agent cwd

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// itemDirSandbox builds a home-as-cwd layout: HOME and the config dir both
// inside one temp dir, the config store created, and the caller's cwd = $HOME
// (the widest cwd library.list will accept, so libraryItemRoots is at its
// weakest and the item DIRS are the only thing left).
func itemDirSandbox(t *testing.T) (sandbox, home string) {
	t.Helper()
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	home = filepath.Join(sandbox, "home")
	if err := os.MkdirAll(home, 0o755); err != nil {
		t.Fatal(err)
	}
	setHome(t, home)
	t.Setenv("USERPROFILE", home)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	if err := os.MkdirAll(libraryGlobalDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	resetCwdCacheForTest()
	return sandbox, home
}

func mustSymlink(t *testing.T, target, link string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(link), 0o755); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, target, link)
}

func TestLibraryItemDirsRefuseEveryShapeOfTheComparison(t *testing.T) {
	for _, tc := range []struct {
		name string
		// setup plants the escape and returns the DERIVED path the service would
		// compose (the string assertLibraryItemPath is handed).
		setup func(t *testing.T, home string) string
		why   string
	}{
		{
			name: "an item directory is compared lexically, never resolved",
			setup: func(t *testing.T, home string) string {
				if err := os.MkdirAll(filepath.Join(home, ".ssh"), 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(filepath.Join(home, ".ssh", "id_rsa.md"),
					[]byte("-----BEGIN OPENSSH PRIVATE KEY-----\n"), 0o600); err != nil {
					t.Fatal(err)
				}
				mustSymlink(t, filepath.Join(home, ".ssh"), filepath.Join(home, ".workspacer", "library"))
				return filepath.Join(home, ".workspacer", "library", "id_rsa.md")
			},
			why: "canonicalizing the project item dirs resolves the very link the gate exists to see, so the derived file lands 'inside' a library directory that is really ~/.ssh",
		},
		{
			name: "a sibling whose name starts with an item directory's is not inside it",
			setup: func(t *testing.T, home string) string {
				if err := os.WriteFile(filepath.Join(home, ".claude.json"),
					[]byte("CLAUDE-OAUTH-ACCOUNT-BLOB"), 0o600); err != nil {
					t.Fatal(err)
				}
				mustSymlink(t, filepath.Join(home, ".claude.json"),
					filepath.Join(home, ".workspacer", "library", "a.md"))
				return filepath.Join(home, ".workspacer", "library", "a.md")
			},
			why: "dropping the separator boundary (strings.HasPrefix instead of containsPath) makes <cwd>/.claude.json a member of the <cwd>/.claude item directory",
		},
		{
			name: "an ancestor of an item directory is not inside it",
			setup: func(t *testing.T, home string) string {
				mustSymlink(t, home, filepath.Join(home, ".claude", "skills", "boom"))
				return filepath.Join(home, ".claude", "skills", "boom")
			},
			why: "flipping the argument order makes every ANCESTOR of an item dir pass — and library.remove's sink is os.RemoveAll, so the whole agent cwd goes",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, home := itemDirSandbox(t)
			derived := tc.setup(t, home)
			canonicalCwd, err := canonicalizePath(home)
			if err != nil {
				t.Fatal(err)
			}
			got, err := assertLibraryItemPath("library.list", derived, canonicalCwd)
			if err == nil {
				t.Fatalf("assertLibraryItemPath(%q) returned %q, want a refusal — %s", derived, got, tc.why)
			}
		})
	}
}

// The floor: the three legitimate item directories still pass, and the value
// returned is the CANONICAL path (BINDING DECISION 2), not the composed one.
// Without this, "refuse everything" would satisfy the cases above.
func TestLibraryItemDirsStillAdmitTheRealItemDirectories(t *testing.T) {
	_, home := itemDirSandbox(t)
	canonicalCwd, err := canonicalizePath(home)
	if err != nil {
		t.Fatal(err)
	}
	for _, p := range []string{
		filepath.Join(libraryGlobalDir(), "ok.md"),
		filepath.Join(home, ".workspacer", "library", "ok.md"),
		filepath.Join(home, ".claude", "skills", "ok", "SKILL.md"),
	} {
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte("# ok"), 0o644); err != nil {
			t.Fatal(err)
		}
		got, err := assertLibraryItemPath("library.list", p, canonicalCwd)
		if err != nil {
			t.Fatalf("assertLibraryItemPath(%q) refused a legitimate item: %v", p, err)
		}
		want, err := canonicalizePath(p)
		if err != nil {
			t.Fatal(err)
		}
		if got != want {
			t.Fatalf("assertLibraryItemPath(%q) = %q, want the canonical path %q", p, got, want)
		}
	}
}

// The same two escapes end to end, through the handler a bus client actually
// calls — the direct test above pins the predicate, this one pins that
// library.list is still wired to it and that no leg bypasses the item dirs.
func TestLibraryListDoesNotLaunderHomeFilesThroughAnItemDirectory(t *testing.T) {
	for _, tc := range []struct {
		name   string
		setup  func(t *testing.T, home string)
		needle string
	}{
		{
			name: "a project library directory symlinked at ~/.ssh",
			setup: func(t *testing.T, home string) {
				if err := os.MkdirAll(filepath.Join(home, ".ssh"), 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(filepath.Join(home, ".ssh", "id_rsa.md"),
					[]byte("-----BEGIN OPENSSH PRIVATE KEY-----\nSTOLEN-KEY\n"), 0o600); err != nil {
					t.Fatal(err)
				}
				mustSymlink(t, filepath.Join(home, ".ssh"), filepath.Join(home, ".workspacer", "library"))
			},
			needle: "STOLEN-KEY",
		},
		{
			name: "an item symlinked at the .claude item dir's prefix-colliding sibling",
			setup: func(t *testing.T, home string) {
				if err := os.WriteFile(filepath.Join(home, ".claude.json"),
					[]byte("CLAUDE-OAUTH-ACCOUNT-BLOB"), 0o600); err != nil {
					t.Fatal(err)
				}
				mustSymlink(t, filepath.Join(home, ".claude.json"),
					filepath.Join(home, ".workspacer", "library", "a.md"))
			},
			needle: "CLAUDE-OAUTH-ACCOUNT-BLOB",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, home := itemDirSandbox(t)
			tc.setup(t, home)

			reg := newRegistry(nil)
			res, err := reg.handle(context.Background(), "library.list",
				json.RawMessage(`{"cwd":`+jsonStr(home)+`}`))
			if err != nil {
				return // a refusal is a perfectly good answer
			}
			if strings.Contains(string(res), tc.needle) {
				t.Fatalf("library.list handed back a file outside every library directory: %s", res)
			}
		})
	}
}
