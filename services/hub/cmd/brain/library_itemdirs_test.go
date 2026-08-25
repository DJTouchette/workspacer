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

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
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

// ---------------------------------------------------------------------------
// contracts/path-containment-cases.json → libraryItemDirs
//
// The hand-written cases above pin THIS copy. This loader pins the copy on the
// other side of the same call with the same cases: the desktop's
// hubCapabilities.ts had libraryItemRoots and no item-DIRECTORY test, so a
// library.list with cwd=$HOME and a `.workspacer/library -> ~/.ssh` symlink was
// refused here and served there — and the desktop is the copy the
// DELEGATE_CATALOG_TO_BRAIN kill switch puts back on the bus. Agreement between
// two providers about one call is not something either side's own tests can
// see, which is what the shared fixture is for.
//
// TWIN LOADER: apps/desktop/src/main/services/hubCapabilitiesKillSwitch.test.ts,
// "library item directories — cross-language contract".
// ---------------------------------------------------------------------------

type libraryItemDirCase struct {
	Name string `json:"name"`
	// Cwd is the caller's directory, sandbox-relative. The loader canonicalizes
	// it exactly as the handler does before the gate is reached — passing the
	// raw string would test a different function.
	Cwd  string `json:"cwd"`
	Item string `json:"item"`
	// Expect is accept|refuse; RefusedBy names WHICH of the two halves refused,
	// recomputed by an oracle below that shares no code with the gate. A bare
	// "refuse" is satisfied by a gate that refuses everything, and the accept
	// cases alone cannot say whether the roots half or the directory half did
	// the work.
	Expect        string       `json:"expect"`
	RefusedBy     string       `json:"refusedBy"`
	ResolvesTo    string       `json:"resolvesTo"`
	NeedsSymlinks bool         `json:"needsSymlinks"`
	Tree          contractTree `json:"tree"`
	Why           string       `json:"why"`
}

type libraryItemDirFixture struct {
	LibraryItemDirs struct {
		Cases []libraryItemDirCase `json:"cases"`
	} `json:"libraryItemDirs"`
}

// libraryItemRefusalReason recomputes which half refused, WITHOUT calling
// either half: containment is decided with filepath.EvalSymlinks and a plain
// prefix test, so a bug in canonicalizePath or containsPath cannot talk this
// oracle into agreeing with the thing it is checking.
func libraryItemRefusalReason(item, canonicalCwd string) string {
	real, err := filepath.EvalSymlinks(item)
	if err != nil {
		return "outside-item-roots" // unresolvable never reaches the dirs test
	}
	under := func(root string) bool {
		rr, err := filepath.EvalSymlinks(root)
		if err != nil {
			return false
		}
		return real == rr || strings.HasPrefix(real, strings.TrimSuffix(rr, string(filepath.Separator))+string(filepath.Separator))
	}
	inRoots := under(libraryGlobalDir()) || under(canonicalCwd)
	if !inRoots {
		return "outside-item-roots"
	}
	// LEXICAL, matching the gate's own deliberate choice: the two cwd-derived
	// directories are compared as written, never resolved.
	for _, dir := range []string{
		filepath.Join(canonicalCwd, ".workspacer", "library"),
		filepath.Join(canonicalCwd, ".claude"),
	} {
		if real == dir || strings.HasPrefix(real, dir+string(filepath.Separator)) {
			return ""
		}
	}
	if under(libraryGlobalDir()) {
		return ""
	}
	return "outside-item-dirs"
}

func TestLibraryItemDirContractCases(t *testing.T) {
	raw := readContractFixtureBytes(t)
	var fx libraryItemDirFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", contractFixtureRel, err)
	}
	cases := fx.LibraryItemDirs.Cases
	if len(cases) == 0 {
		t.Fatalf("%s decoded to zero libraryItemDirs cases — a silently empty corpus guards nothing", contractFixtureRel)
	}

	// EXECUTED, not enumerated: a needsSymlinks case skips itself on a host
	// without the privilege, and counting the fixture's length instead would
	// report a full sweep for a run that asserted nothing.
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
			for _, d := range []string{"home", "outside", filepath.Join("config", "workspacer", "library")} {
				if err := os.MkdirAll(filepath.Join(sandbox, d), 0o755); err != nil {
					t.Fatal(err)
				}
			}
			setHome(t, filepath.Join(sandbox, "home"))
			t.Setenv("USERPROFILE", filepath.Join(sandbox, "home"))
			setConfigHome(t, configHome)
			t.Setenv("APPDATA", configHome)
			resetCwdCacheForTest()

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

			canonicalCwd, err := canonicalizePath(filepath.Join(sandbox, c.Cwd))
			if err != nil {
				t.Fatalf("canonicalize the case cwd %q: %v", c.Cwd, err)
			}
			item := filepath.Join(sandbox, c.Item)
			got, err := assertLibraryItemPath("library.list", item, canonicalCwd)

			if c.Expect == "accept" {
				if err != nil {
					t.Fatalf("assertLibraryItemPath(%q) refused a legitimate item: %v\n  why: %s", item, err, c.Why)
				}
				want := filepath.Join(sandbox, c.ResolvesTo)
				if got != want {
					t.Fatalf("assertLibraryItemPath(%q) = %q, want %q — the string that was checked must be the string that is opened\n  why: %s",
						item, got, want, c.Why)
				}
				return
			}
			if err == nil {
				t.Fatalf("assertLibraryItemPath(%q) returned %q, want a refusal\n  why: %s", item, got, c.Why)
			}
			if reason := libraryItemRefusalReason(item, canonicalCwd); reason != c.RefusedBy {
				t.Fatalf("refused for the WRONG REASON: the oracle says %q, the fixture says %q\n  item: %q\n  why: %s",
					reason, c.RefusedBy, item, c.Why)
			}
		})
	}

	// RequireCorpus, not Require: the enumerated floor is what catches a corpus
	// that SHRANK (host-independent), while the two verdict floors answer "did
	// this host prove anything about each class" and stay on what executed. Both
	// are derived from the fixture, so adding a case raises the floor with it and
	// deleting one goes red.
	var wantAllow, wantDeny int
	for _, c := range cases {
		if c.Expect == "accept" {
			wantAllow++
		} else if !c.NeedsSymlinks {
			wantDeny++
		}
	}
	if err := tally.RequireCorpus("the libraryItemDirs corpus sweep", len(cases), wantAllow, wantDeny); err != nil {
		t.Fatal(err)
	}
	t.Logf("libraryItemDirs: %s", tally.String())
}
