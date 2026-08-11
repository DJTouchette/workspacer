package extinput_test

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/extinput"
)

// TestPathStaysLexicallyInsideTheModuleRoot checks the one property
// search.InDir tests. It is the cheap half of the guard; the expensive half
// (TestTheCachePinActuallyMisses) proves cmd/go agrees.
func TestPathStaysLexicallyInsideTheModuleRoot(t *testing.T) {
	root, err := extinput.ModuleRoot()
	if err != nil {
		t.Fatalf("module root: %v", err)
	}
	// The real thing this repo reads: a file four levels above services/hub.
	const twin = "../../../../apps/desktop/src/main/services/hubCapabilities.ts"
	pinned, err := extinput.Path(twin)
	if err != nil {
		t.Fatalf("Path(%s): %v", twin, err)
	}
	if !strings.HasPrefix(pinned, root+string(filepath.Separator)) {
		t.Fatalf("Path(%s) = %s, which does not descend from the module root %s — search.InDir is a string-prefix test, so cmd/go would drop this input from the test cache key and every cross-repo guard would go back to passing on bytes it never read", twin, pinned, root)
	}
	if clean := filepath.Clean(pinned); strings.HasPrefix(clean, root+string(filepath.Separator)) {
		t.Fatalf("Path(%s) = %s cleans to %s, still under the module root — this test is asserting nothing, because the file it names is supposed to live OUTSIDE the module", twin, pinned, clean)
	}
	// And it must still name the real file.
	abs, err := filepath.Abs(twin)
	if err != nil {
		t.Fatalf("abs: %v", err)
	}
	di, derr := os.Stat(abs)
	pi, perr := os.Stat(pinned)
	if derr != nil || perr != nil {
		t.Skipf("not a monorepo checkout (%v / %v); the pinned-path shape above still held", derr, perr)
	}
	if !os.SameFile(di, pi) {
		t.Fatalf("%s and its pinned form %s are different files", abs, pinned)
	}
}

// TestTheCachePinActuallyMisses is the guard on the guard, and the reason this
// file is long.
//
// The previous pin (an os.Getenv on a name carrying the file's content hash)
// looked exactly as convincing as this one and did nothing: `go test` replays
// the RECORDED name against the CURRENT value, so a name minted by the last run
// re-hashes to the same bytes forever. Nothing in the Go type system, and
// nothing short of running cmd/go, can tell a working pin from that. So this
// test runs cmd/go.
//
// It builds a miniature of this monorepo in a temp dir — Makefile and
// services/hub/go.mod markers, a contracts/ fixture ABOVE the module root, and
// verbatim copies of extinput.go and sweepguard.go — then runs `go test` three
// times over three packages:
//
//	pinned    reads the external file via sweepguard.ReadRepoFile   (the real chain)
//	pinneddir lists an external directory via extinput.ReadDir
//	plain     reads the external file via os.ReadFile               (the control)
//
// Run 2 must report all three cached: without that, "run 3 was a miss" would
// prove only that something invalidates everything. Run 3, after the external
// file changes, must report `plain` STILL cached — that is the bug, reproduced
// on demand, and it is what makes the other two misses meaningful.
func TestTheCachePinActuallyMisses(t *testing.T) {
	if testing.Short() {
		t.Skip("runs cmd/go three times; -short skips it")
	}
	goBin, err := exec.LookPath("go")
	if err != nil {
		t.Skipf("no `go` on PATH, so cmd/go's cache behaviour cannot be observed: %v", err)
	}

	root := t.TempDir()
	scratch := buildScratchRepo(t, root)

	extFile := filepath.Join(root, "contracts", "fixture.json")
	extDir := filepath.Join(root, "contracts", "cases")

	writeAged(t, extFile, `{"v":1}`)
	writeAged(t, filepath.Join(extDir, "a.json"), `{"a":1}`)

	// Run 1 populates the cache. Run 2 must be a clean sweep of hits.
	runScratch(t, goBin, scratch)
	hits := runScratch(t, goBin, scratch)
	for _, pkg := range []string{"pinned", "pinneddir", "plain"} {
		if !hits[pkg] {
			t.Fatalf("run 2: %s was not cached (%v) — nothing below can be attributed to the pin if an unchanged tree already misses", pkg, hits)
		}
	}

	// The mutation: the cross-repo file changes, and a new case file appears.
	writeAged(t, extFile, `{"v":2,"added":"an unclassified param"}`)
	writeAged(t, filepath.Join(extDir, "b.json"), `{"b":2}`)

	after := runScratch(t, goBin, scratch)
	if after["plain"] != true {
		t.Fatalf("control failed: `plain` (os.ReadFile of an out-of-module path) was NOT cached after the external edit (%v) — either cmd/go now re-checks out-of-root inputs on its own, in which case this package is obsolete, or something else in the scratch tree changed and the two assertions below prove nothing", after)
	}
	if after["pinned"] {
		t.Fatalf("sweepguard.ReadRepoFile -> extinput.ReadFile reported `ok (cached)` after its cross-repo file changed (%v) — the pin is a no-op again, and every TS<->Go drift guard in this repo is now green on trees it never read", after)
	}
	if after["pinneddir"] {
		t.Fatalf("extinput.ReadDir reported `ok (cached)` after a new file appeared in the external directory (%v) — an enumeration guard that cannot see a new fixture is not a guard", after)
	}
}

// runScratch runs `go test ./...` in the scratch module and reports, per
// package, whether cmd/go said "(cached)".
func runScratch(t *testing.T, goBin, dir string) map[string]bool {
	t.Helper()
	cmd := exec.Command(goBin, "test", "./...")
	cmd.Dir = dir
	// A deliberately sterile environment: GOFLAGS could carry the very
	// -count=1 this test needs absent, GOWORK could drag in the real
	// workspace, and GOPROXY must never be consulted (the scratch module has
	// no dependencies). GOCACHE is inherited on purpose — a cold build cache
	// would make this test compile the standard library.
	cmd.Env = append(os.Environ(), "GOFLAGS=", "GOWORK=off", "GO111MODULE=on", "GOPROXY=off")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("scratch `go test ./...` failed: %v\n%s", err, out)
	}
	cached := map[string]bool{}
	for _, line := range strings.Split(string(out), "\n") {
		if !strings.HasPrefix(line, "ok") {
			continue
		}
		for _, pkg := range []string{"pinned", "pinneddir", "plain"} {
			if strings.Contains(line, "extinputselftest/"+pkg+"\t") {
				cached[pkg] = strings.Contains(line, "(cached)")
			}
		}
	}
	if len(cached) != 3 {
		t.Fatalf("scratch run reported %d of 3 packages (%v):\n%s", len(cached), cached, out)
	}
	return cached
}

// writeAged writes a file and backdates its mtime an hour. cmd/go refuses to
// cache a result whose input file is younger than modTimeCutoff (2s), so
// without this every assertion above would need a sleep and the honest ones
// would be indistinguishable from the too-new refusal.
func writeAged(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-time.Hour)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatal(err)
	}
}

// buildScratchRepo lays out the miniature monorepo and returns its Go module
// directory. extinput.go and sweepguard.go are COPIED, not reimplemented: a
// hand-written stand-in would keep passing after the real ones regressed.
func buildScratchRepo(t *testing.T, root string) string {
	t.Helper()

	realModule := modulePathFromGoMod(t, filepath.Join("..", "..", "go.mod"))
	const scratchModule = "extinputselftest"

	hub := filepath.Join(root, "services", "hub")
	write(t, filepath.Join(root, "Makefile"), "# monorepo root marker for sweepguard.Root\n")
	write(t, filepath.Join(hub, "go.mod"), "module "+scratchModule+"\n\ngo 1.21\n")

	copyGo(t, "extinput.go", filepath.Join(hub, "internal", "extinput", "extinput.go"), realModule, scratchModule)
	copyGo(t, filepath.Join("..", "sweepguard", "sweepguard.go"), filepath.Join(hub, "internal", "sweepguard", "sweepguard.go"), realModule, scratchModule)

	write(t, filepath.Join(hub, "pinned", "pinned_test.go"), `package pinned

import (
	"testing"

	"`+scratchModule+`/internal/sweepguard"
)

func TestReadsTheExternalFixture(t *testing.T) {
	if _, err := sweepguard.ReadRepoFile("contracts", "fixture.json"); err != nil {
		t.Fatal(err)
	}
}
`)

	write(t, filepath.Join(hub, "pinneddir", "pinneddir_test.go"), `package pinneddir

import (
	"testing"

	"`+scratchModule+`/internal/extinput"
	"`+scratchModule+`/internal/sweepguard"
)

func TestListsTheExternalCases(t *testing.T) {
	dir, err := sweepguard.RepoPath("contracts", "cases")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := extinput.ReadDir(dir); err != nil {
		t.Fatal(err)
	}
}
`)

	write(t, filepath.Join(hub, "plain", "plain_test.go"), `package plain

import (
	"os"
	"path/filepath"
	"testing"
)

// The control: exactly what every cross-repo reader in this repo did before
// extinput, and what it must keep doing here so the pinned packages' misses
// mean something.
func TestReadsTheExternalFixtureUnpinned(t *testing.T) {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	p := filepath.Join(wd, "..", "..", "..", "contracts", "fixture.json")
	if _, err := os.ReadFile(p); err != nil {
		t.Fatal(err)
	}
}
`)

	return hub
}

func modulePathFromGoMod(t *testing.T, path string) string {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	for _, line := range strings.Split(string(raw), "\n") {
		if rest, ok := strings.CutPrefix(strings.TrimSpace(line), "module "); ok {
			return strings.TrimSpace(rest)
		}
	}
	t.Fatalf("no module line in %s", path)
	return ""
}

// copyGo copies a source file into the scratch module, rewriting this module's
// import path to the scratch one. internal/ packages cannot be imported across
// modules, which is why this is a copy and not a `replace`.
func copyGo(t *testing.T, src, dst, realModule, scratchModule string) {
	t.Helper()
	raw, err := os.ReadFile(src)
	if err != nil {
		t.Fatalf("read %s: %v", src, err)
	}
	body := strings.ReplaceAll(string(raw), realModule+"/", scratchModule+"/")
	if strings.Contains(body, realModule) {
		t.Fatalf("%s still imports %s after rewriting; the scratch module would not build", src, realModule)
	}
	write(t, dst, body)
}

func write(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

// TestWalkDirTraversesAndPrunes covers [extinput.WalkDir]'s traversal contract.
// The CACHE property it adds is inherited from ReadDir, which
// TestTheCachePinActuallyMisses proves against cmd/go itself; what is new here
// is the recursion, and a walk that silently visited only the top level would
// pin only the top level while looking like it pinned the tree.
func TestWalkDirTraversesAndPrunes(t *testing.T) {
	// INSIDE the module, not os.TempDir(). extinput maps every path it pins to a
	// module-relative name, and on the Windows CI runner the checkout is on D:
	// while TEMP is on C: — filepath.Rel across volumes fails, which the
	// production code correctly reports rather than papering over. The test has
	// to hand it a path it can express; the cache property under test is
	// unaffected by where the tree lives.
	root := moduleTempDir(t)
	for _, rel := range []string{
		filepath.Join("a.txt"),
		filepath.Join("sub", "b.txt"),
		filepath.Join("sub", "deep", "c.txt"),
		filepath.Join("skipme", "d.txt"),
	} {
		write(t, filepath.Join(root, rel), "x")
	}

	var seen []string
	err := extinput.WalkDir(root, func(path string, d os.DirEntry) error {
		rel, relErr := filepath.Rel(root, path)
		if relErr != nil {
			return relErr
		}
		if d.IsDir() && d.Name() == "skipme" {
			return filepath.SkipDir
		}
		if !d.IsDir() {
			seen = append(seen, filepath.ToSlash(rel))
		}
		return nil
	})
	if err != nil {
		t.Fatalf("WalkDir: %v", err)
	}
	sort.Strings(seen)
	want := []string{"a.txt", "sub/b.txt", "sub/deep/c.txt"}
	if strings.Join(seen, ",") != strings.Join(want, ",") {
		t.Fatalf("WalkDir visited %v, want %v — a walk that stops at the top level pins only the top level's listing while reading as a whole-tree guard", seen, want)
	}

	// A root that is not there is the caller looking in the wrong place, and
	// must be an error rather than an empty, successful walk: an empty walk is
	// how "every fixture has two loaders" would pass by finding no loaders and
	// no fixtures.
	if err := extinput.WalkDir(filepath.Join(root, "nope"), func(string, os.DirEntry) error { return nil }); err == nil {
		t.Fatal("WalkDir over a missing root returned nil — a guard that walks the wrong path would report a clean sweep of nothing")
	}

	// An error from visit propagates, from any depth.
	sentinel := errors.New("boom")
	err = extinput.WalkDir(root, func(path string, d os.DirEntry) error {
		if d.Name() == "c.txt" {
			return sentinel
		}
		return nil
	})
	if !errors.Is(err, sentinel) {
		t.Fatalf("WalkDir swallowed a visit error from depth 2: %v", err)
	}
}

// moduleTempDir makes a scratch tree inside the module and removes it after the
// test. See TestWalkDirTraversesAndPrunes for why os.TempDir() will not do.
func moduleTempDir(t *testing.T) string {
	t.Helper()
	dir, err := os.MkdirTemp(".", "walkdir-")
	if err != nil {
		t.Fatalf("temp dir inside the module: %v", err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	abs, err := filepath.Abs(dir)
	if err != nil {
		t.Fatalf("abs: %v", err)
	}
	return abs
}
