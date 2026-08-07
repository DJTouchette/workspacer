package main

// contracts/filename-slug-cases.json — the cross-language filename corpus.
//
// stores_test.go's TestSlugs is a characterization test of THIS copy, and
// apps/desktop/src/main/lib/fileUtils.test.ts is a characterization test of the
// other one. Neither ever saw the other's answers, which is exactly how the two
// came to write different filenames into the same config store: Go's
// strings.ToLower folds U+0130 to a single 'i' while JavaScript's toLowerCase
// folds it to 'i' + U+0307, so `layouts.save {name: "aİb"}` produced aib.yaml
// here and ai-b.yaml in the app — and which side answers depends on
// DELEGATE_CATALOG_TO_BRAIN. Unlike pricing, deepMerge and path containment,
// nothing in contracts/ pinned the slugs. This is one of the fixture's two
// loaders; the other is fileUtils.test.ts.

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// slugFixtureRel is relative to this package dir (services/hub/cmd/brain).
const slugFixtureRel = "../../../../contracts/filename-slug-cases.json"

// slugOwnerKey is this implementation's key in the fixture's `owners` map.
const slugOwnerKey = "services/hub/cmd/brain/slug.go"

type slugCase struct {
	Name   string            `json:"name"`
	Input  string            `json:"input"`
	Expect map[string]string `json:"expect"`
	Why    string            `json:"why"`
}

type slugFixture struct {
	Owners map[string][]string `json:"owners"`
	Cases  []slugCase          `json:"cases"`
}

// slugCorpusFloor is the size of the corpus today. See fsguard_test.go's floors:
// a floor of "not zero" is met by a corpus that lost 16 of its 17 cases.
const slugCorpusFloor = 17

func TestFilenameSlugContractCases(t *testing.T) {
	raw, err := os.ReadFile(slugFixtureRel)
	if err != nil {
		t.Fatalf("read %s: %v", slugFixtureRel, err)
	}
	var fx slugFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", slugFixtureRel, err)
	}
	// Dropping this file out of `owners` would otherwise run every case and
	// prove nothing about whether this copy is still on the hook.
	variants := map[string]bool{}
	for _, v := range fx.Owners[slugOwnerKey] {
		variants[v] = true
	}
	for _, required := range []string{"library", "layout", "session"} {
		if !variants[required] {
			t.Fatalf("owners[%s] must include %q; got %v", slugOwnerKey, required, fx.Owners[slugOwnerKey])
		}
	}
	if len(fx.Cases) == 0 {
		t.Fatalf("%s decoded to zero cases — a silently empty corpus guards nothing", slugFixtureRel)
	}

	fns := map[string]func(string) string{
		"library": slugLibrary,
		"layout":  slugLayout,
		"session": slugSession,
	}
	// The floor. `len(fx.Cases) == 0` above catches a corpus that vanished; it
	// does not catch one that SHRANK, and it does not notice a case that
	// registered and then never asserted. The tally counts inside the body.
	var tally sweepguard.Tally
	for _, c := range fx.Cases {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
			for variant, fn := range fns {
				want, ok := c.Expect[variant]
				if !ok {
					t.Fatalf("case %q has no expectation for the %q variant — every case must pin all three, or a variant silently stops being covered", c.Name, variant)
				}
				if got := fn(c.Input); got != want {
					t.Errorf("slug%s(%q) = %q, want %q\n  why: %s", variant, c.Input, got, want, c.Why)
				}
			}
			// remove()/delete re-slug a STORED id, so a non-idempotent variant
			// unlinks a filename save() never wrote — the same "undeletable
			// item" failure the U+0130 divergence produced, from the other
			// direction. library and layout both trim and both re-trim after
			// truncation for exactly this reason; session deliberately does not
			// trim, so only the two trimming variants are held to it.
			for _, variant := range []string{"library", "layout"} {
				fn := fns[variant]
				if once, twice := fn(c.Input), fn(fns[variant](c.Input)); once != twice {
					t.Errorf("slug%s is not idempotent on %q: %q then %q", variant, c.Input, once, twice)
				}
			}
		})
	}
	if err := tally.RequireEvery("the filename-slug corpus", slugCorpusFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}
