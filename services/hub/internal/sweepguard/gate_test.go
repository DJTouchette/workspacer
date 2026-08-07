package sweepguard

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The ratchet is the half RequireBoth does not have. A floor of one is satisfied
// by a corpus that lost 98% of its cases, which is the slow version of the empty
// sweep this package was written for.
func TestRequireCorpusCatchesAShrunkCorpus(t *testing.T) {
	full := &Tally{Allow: 40, Deny: 39}
	if err := full.RequireCorpus("the containment corpus", 79, 1, 1); err != nil {
		t.Fatalf("a full corpus must pass its own floor: %v", err)
	}

	shrunk := &Tally{Allow: 1, Deny: 1}
	if err := shrunk.RequireBoth("the containment corpus"); err != nil {
		t.Fatal("precondition: RequireBoth is happy with 2 cases — that is the hole")
	}
	err := shrunk.RequireCorpus("the containment corpus", 79, 1, 1)
	if err == nil {
		t.Fatal("RequireCorpus accepted a corpus that shrank from 79 cases to 2")
	}
	for _, want := range []string{"reached 2 cases", "floor is 79", "SHRANK"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("the failure must name the shrink and the floor; %q is missing from: %s", want, err)
		}
	}

	// SKIPPED cases count toward the enumeration floor, and that is the point:
	// a host without symlink privilege skips half the corpus and must still pass
	// this check, or the ratchet would be a machine-dependent flake and the next
	// person would lower it to 1.
	skippy := &Tally{Allow: 1, Deny: 1}
	for i := 0; i < 77; i++ {
		skippy.Skip("needsSymlinks")
	}
	if err := skippy.RequireCorpus("the containment corpus", 79, 1, 1); err != nil {
		t.Fatalf("a host that skipped 77 of 79 cases still ENUMERATED 79: %v", err)
	}
	if skippy.Enumerated() != 79 {
		t.Fatalf("Enumerated()=%d, want 79", skippy.Enumerated())
	}
}

// A GateCounter that could be satisfied by registration, or by a group that
// shrank, would be the counter it replaces.
func TestGateCounterRequiresEveryDeclaredTest(t *testing.T) {
	g := Gate("the symlink-gated tests", 3)
	if err := g.Require(); err == nil {
		t.Fatal("a gate that ran nothing must be an error — that is the whole failure mode")
	}
	g.Ran("TestA")
	g.Ran("TestA") // the same test creating a second symlink is still one test
	g.Ran("TestB")
	g.Skip("TestC", "needsSymlinks")
	err := g.Require()
	if err == nil {
		t.Fatal("2 of 3 must fail: a group that quietly shrinks is the same hole arriving more slowly")
	}
	if !strings.Contains(err.Error(), "TestC (needsSymlinks)") {
		t.Errorf("the failure must name the test that did not run and why: %s", err)
	}
	g.Ran("TestC")
	if err := g.Require(); err != nil {
		t.Fatalf("a fully executed group must pass: %v", err)
	}
	if g.Count() != 3 {
		t.Fatalf("Count()=%d, want 3", g.Count())
	}

	g.Ran("TestD")
	if err := g.Require(); err == nil || !strings.Contains(err.Error(), "ADDED") {
		t.Fatalf("adding a test without raising the count must fail loudly; got %v", err)
	}
}

// The two orderings, and the second is the one with teeth: a test that clears
// the symlink gate and then skips for an unrelated reason (no git, a filename
// this filesystem will not hold) asserted nothing, and a counter that kept it as
// "ran" would be the enumeration lie again with extra steps.
func TestGateCounterTakesTheLastVerdict(t *testing.T) {
	g := Gate("ordering", 1)
	g.Skip("TestA", "needsSymlinks")
	g.Ran("TestA")
	if err := g.Require(); err != nil {
		t.Fatalf("Ran must clear an earlier Skip for the same test: %v", err)
	}
	g.Skip("TestA", "skipped after clearing the gate")
	if err := g.Require(); err == nil {
		t.Fatal("a test that skipped AFTER clearing its gate ran nothing, and must not stay counted as run")
	}
}

// Renaming the root Makefile used to answer ErrNoCheckout, which every caller
// treats as "vendored, nothing to check" — silently standing down every
// cross-repo guard in the repo. A partial marker match is a MOVED MARKER and has
// to be as loud as a moved fixture.
func TestRootIsLoudWhenOnlySomeMarkersArePresent(t *testing.T) {
	fake := t.TempDir()
	if err := os.MkdirAll(filepath.Join(fake, "services", "hub"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(fake, "services", "hub", "go.mod"), []byte("module x\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// No Makefile: exactly what `git mv Makefile build.mk` leaves behind.
	deep := filepath.Join(fake, "services", "hub", "cmd", "brain")
	if err := os.MkdirAll(deep, 0o755); err != nil {
		t.Fatal(err)
	}
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chdir(deep); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chdir(wd) })

	_, err = Root()
	if err == nil {
		t.Fatal("Root() accepted a tree with a missing marker")
	}
	if errors.Is(err, ErrNoCheckout) {
		t.Fatalf("a monorepo with a RENAMED marker was reported as a vendored checkout — callers skip on ErrNoCheckout, so this is how renaming the Makefile turns ~17 cross-repo guards off in silence: %v", err)
	}
	if !strings.Contains(err.Error(), "Makefile") {
		t.Errorf("the failure must name the marker that is missing: %v", err)
	}
	if _, err := ReadRepoFile("contracts", "path-containment-cases.json"); errors.Is(err, ErrNoCheckout) {
		t.Fatal("ReadRepoFile inherited the wrong verdict: a renamed marker must not read as ErrNoCheckout")
	}
}
