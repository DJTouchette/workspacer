package sweepguard

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The floor is the product. If Require could return nil for a sweep that ran
// nothing, every loader that calls it would be back to a green PASS over zero
// executed cases — the exact failure this package exists to make impossible.
func TestRequireFailsOnAnEmptyOrHalfEmptySweep(t *testing.T) {
	var empty Tally
	if err := empty.RequireBoth("corpus"); err == nil {
		t.Fatal("RequireBoth accepted a sweep that executed nothing")
	}

	allowOnly := &Tally{Allow: 42}
	err := allowOnly.RequireBoth("corpus")
	if err == nil {
		t.Fatal("RequireBoth accepted a sweep that ran 42 allow cases and zero denies — that proves a guard lets things through and nothing else")
	}
	if !strings.Contains(err.Error(), "0 deny") {
		t.Fatalf("the failure must name the missing verdict class; got %q", err)
	}

	denyOnly := &Tally{Deny: 7}
	if err := denyOnly.RequireBoth("corpus"); err == nil {
		t.Fatal("RequireBoth accepted a sweep with zero allow cases — a handler that refuses everything satisfies a deny-only sweep")
	}
	if err := denyOnly.RequireDeny("deny-only probe"); err != nil {
		t.Fatalf("RequireDeny must accept a deny-only sweep that ran denies: %v", err)
	}
	if err := (&Tally{}).RequireDeny("deny-only probe"); err == nil {
		t.Fatal("RequireDeny accepted zero deny cases")
	}

	both := &Tally{Allow: 1, Deny: 1}
	if err := both.RequireBoth("corpus"); err != nil {
		t.Fatalf("RequireBoth rejected a sweep that ran both classes: %v", err)
	}
}

// A floor failure has to say WHY the sweep was empty, or the operator's next
// move is a bisect instead of "turn on developer mode".
func TestSkipReasonsAppearInTheFailure(t *testing.T) {
	var ta Tally
	for i := 0; i < 3; i++ {
		ta.Skip("needsSymlinks")
	}
	ta.Skip("needsHome")
	ta.Ran("allow")

	err := ta.RequireBoth("corpus")
	if err == nil {
		t.Fatal("expected a floor failure")
	}
	for _, want := range []string{"needsSymlinks×3", "needsHome×1", "4 case(s) skipped"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("failure text is missing %q: %s", want, err)
		}
	}
}

// The verdict words differ per fixture block (allow/deny in the containment
// corpus, accept/refuse in sessionFilenames). Filing them apart is what makes
// the two floors independent; a Tally that lumped them into Other would report
// "0 allow, 0 deny" for a fully executed sweep and fail every loader.
func TestVerdictVocabulary(t *testing.T) {
	var ta Tally
	ta.Ran("allow")
	ta.Ran("accept")
	ta.Ran("deny")
	ta.Ran("refuse")
	ta.Ran("whatever")
	if ta.Allow != 2 || ta.Deny != 2 || ta.Other != 1 {
		t.Fatalf("allow=%d deny=%d other=%d, want 2/2/1", ta.Allow, ta.Deny, ta.Other)
	}
	if ta.Executed() != 5 {
		t.Fatalf("Executed()=%d, want 5", ta.Executed())
	}
}

// Root must find the monorepo from inside a package four levels down, and
// ReadRepoFile must distinguish "no checkout" (an honest skip) from "the
// checkout is here and the file moved" (a failure). Collapsing those two is the
// bug: every cross-repo reader in this repo used to Skipf on both.
func TestReadRepoFileSeparatesAMissingCheckoutFromAMissingFile(t *testing.T) {
	root, err := Root()
	if err != nil {
		t.Fatalf("Root() from %s: %v", mustWD(t), err)
	}
	for _, m := range rootMarkers {
		if _, err := os.Stat(filepath.Join(root, m)); err != nil {
			t.Fatalf("Root() returned %s, which does not carry %s", root, m)
		}
	}

	if _, err := ReadRepoFile("contracts", "path-containment-cases.json"); err != nil {
		t.Fatalf("the containment corpus must be readable from the repo root: %v", err)
	}

	_, err = ReadRepoFile("contracts", "there-is-no-such-fixture.json")
	if err == nil {
		t.Fatal("reading an absent file inside a present checkout must fail")
	}
	if errors.Is(err, ErrNoCheckout) {
		t.Fatal("a missing FILE was reported as a missing CHECKOUT — callers skip on ErrNoCheckout, so this is precisely how a renamed twin turns a guard off silently")
	}

	// And the honest case: run Root() from a directory with no monorepo above
	// it. tmp is not under the checkout, so the walk reaches / and stops.
	dir := t.TempDir()
	restore := mustWD(t)
	if err := os.Chdir(dir); err != nil {
		t.Fatalf("chdir: %v", err)
	}
	t.Cleanup(func() { _ = os.Chdir(restore) })
	if _, err := Root(); !errors.Is(err, ErrNoCheckout) {
		t.Fatalf("Root() outside any checkout = %v, want ErrNoCheckout", err)
	}
}

func mustWD(t *testing.T) string {
	t.Helper()
	wd, err := os.Getwd()
	if err != nil {
		t.Fatalf("getwd: %v", err)
	}
	return wd
}
