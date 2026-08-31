package routing

import (
	"encoding/json"
	"errors"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

func TestSplitModelWindowSuffix(t *testing.T) {
	cases := []struct {
		in     string
		base   string
		suffix string
		why    string
	}{
		{"opus[1m]", "opus", "[1m]", "the composer's own spelling, and the desktop's shipped claude.defaultModel"},
		{"sonnet[1m]", "sonnet", "[1m]", "same for sonnet"},
		{"claude-opus-5[1m]", "claude-opus-5", "[1m]", "a concrete id may carry it too"},
		{"claude-sonnet-5-1m", "claude-sonnet-5", "-1m", "the id-suffix spelling of the same marker"},
		{"OPUS[1M]", "OPUS", "[1M]", "the marker is matched case-insensitively and comes back as it was written"},
		{"  opus[1m]  ", "opus", "[1m]", "surrounding whitespace is not part of either half"},
		{"opus", "opus", "", "no marker is not an empty marker"},
		{"gpt-5.6-sol", "gpt-5.6-sol", "", "a non-claude id carries no window vocabulary"},
		{"fable", "fable", "", "1M-NATIVE, not marked: fable's window is a fact about the model, so there is no suffix to take off and nothing here should invent one"},
		{"-1m", "-1m", "", "a string that is NOTHING but a marker names no model; emptying the base would read as 'the matrix has no opinion', which is far more permissive"},
		{"[1m]", "[1m]", "", "same for the bracket spelling"},
		{"", "", "", "nothing in, nothing out"},
	}
	for _, c := range cases {
		base, suffix := splitModelWindowSuffix(c.in)
		if base != c.base || suffix != c.suffix {
			t.Errorf("splitModelWindowSuffix(%q) = (%q, %q), want (%q, %q): %s", c.in, base, suffix, c.base, c.suffix, c.why)
		}
	}
}

func TestMatchableModelIsTheComparisonForm(t *testing.T) {
	for _, c := range []struct{ a, b string }{
		{"opus[1m]", "opus"},
		{"OPUS[1M]", "opus"},
		{" opus[1m] ", "opus"},
		{"claude-sonnet-5-1m", "claude-sonnet-5"},
	} {
		if matchableModel(c.a) != matchableModel(c.b) {
			t.Errorf("matchableModel(%q)=%q and matchableModel(%q)=%q differ — the window suffix is a request for a bigger window on the SAME model, not another model",
				c.a, matchableModel(c.a), c.b, matchableModel(c.b))
		}
	}
	if matchableModel("opus") == matchableModel("sonnet") {
		t.Error("two different models compare equal — the normalizer is eating more than the suffix")
	}
}

// THE DRIFT GUARD. contracts/model-context-windows.json is where this repo
// decides how a 1M window is SPELLED, and three window tables are already pinned
// to it. The suffixes stripped here have to be those same spellings: rename one
// in the fixture and leave this list behind, and the ceiling goes back to being
// unable to read the desktop's own default under whatever the new spelling is.
//
// It checks membership rather than equality on purpose. The fixture's 1M rows
// also include `fable` and `mythos`, which are 1M-NATIVE families rather than
// markers: there is no suffix to take off them, and nothing here should try.
func TestTheWindowSuffixesAreTheContractsOwnSpellings(t *testing.T) {
	raw, err := sweepguard.ReadRepoFile("contracts", "model-context-windows.json")
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout, so there is no fixture to cross-check against: %v", err)
		}
		t.Fatalf("read contracts/model-context-windows.json: %v", err)
	}
	var fixture struct {
		Windows []struct {
			Match string `json:"match"`
		} `json:"windows"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatalf("parse contracts/model-context-windows.json: %v", err)
	}
	known := map[string]bool{}
	for _, row := range fixture.Windows {
		known[row.Match] = true
	}
	if len(known) == 0 {
		t.Fatal("the fixture's windows block is empty, so this guard read the wrong file")
	}
	for _, s := range windowSuffixes {
		if !known[s] {
			t.Errorf("windowSuffixes carries %q, which contracts/model-context-windows.json's windows block does not list — the routing lookup and the window tables disagree about how a 1M request is spelled", s)
		}
	}
}
