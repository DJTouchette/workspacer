package limits

import (
	"encoding/json"
	"errors"
	"path/filepath"
	"sort"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// The ROUTING half of contracts/usage-window-currency-cases.json. The other
// loader is apps/desktop/src/main/services/keepWarmLogic.test.ts, which holds
// fiveHourWindowFromReport to the same currency verdicts through its own
// narrower three-way return.

var currencyFixtureName = []string{"contracts", "usage-window-currency-cases.json"}

type currencyFixture struct {
	Vocabulary struct {
		RolledOverReasons map[string]string `json:"rolledOverReasons"`
		UnreadableReasons map[string]string `json:"unreadableReasons"`
	} `json:"vocabulary"`
	Cases []currencyCase `json:"cases"`
}

type currencyCase struct {
	Name           string      `json:"name"`
	Now            int64       `json:"now"`
	Window         *WireWindow `json:"window"`
	Expect         string      `json:"expect"`
	UnknownBecause string      `json:"unknownBecause"`
	UsedPercent    *float64    `json:"usedPercent"`
	SecondsToReset *int64      `json:"secondsToReset"`
	Why            string      `json:"why"`
}

// currencyCaseFloor pins the corpus near where it actually is. A fixture edit
// that drops half the cases keeps one of each verdict and would otherwise stay
// green forever.
const currencyCaseFloor = 11

// loadCurrencyFixture reads the corpus through sweepguard -> extinput rather
// than os.ReadFile, for the reason internal/bus/policy_test.go documents at
// length: contracts/ lives four levels ABOVE this module and cmd/go drops every
// test input whose path fails search.InDir. Read straight, this whole sweep
// would report `ok (cached)` after the fixture changed.
func loadCurrencyFixture(t *testing.T) currencyFixture {
	t.Helper()
	raw, err := sweepguard.ReadRepoFile(currencyFixtureName...)
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout, so the shared window-currency corpus has nothing to read: %v", err)
		}
		t.Fatalf("%v", err)
	}
	var fx currencyFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", filepath.ToSlash(filepath.Join(currencyFixtureName...)), err)
	}
	if len(fx.Cases) == 0 {
		t.Fatal("the window-currency corpus decoded to zero cases — a field was renamed and this sweep is asserting nothing")
	}
	return fx
}

// TestWindowCurrencyMatchesTheContract is the sweep.
func TestWindowCurrencyMatchesTheContract(t *testing.T) {
	fx := loadCurrencyFixture(t)
	var tally sweepguard.Tally

	for _, c := range fx.Cases {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran(c.Expect)
			now := time.Unix(c.Now, 0)
			got := ReadWindow(c.Window, Provenance{Source: "disk"}, now)

			if string(got.State) != c.Expect {
				t.Fatalf("state = %q, want %q\n  why: %s", got.State, c.Expect, c.Why)
			}
			if got.Reason != c.UnknownBecause {
				t.Errorf("reason = %q, want %q — a bare verdict is satisfied by any outcome of that class, including one reached for a different reason", got.Reason, c.UnknownBecause)
			}
			if got.Usable() != (c.Expect == "current") {
				t.Errorf("Usable() = %v on a %q reading", got.Usable(), c.Expect)
			}

			// The half that matters: a non-current reading must yield NO
			// number. Not zero, not the stale percentage, not a negative
			// duration — nothing.
			used, haveUsed := got.UsedPercent()
			switch {
			case c.UsedPercent == nil && haveUsed:
				t.Errorf("UsedPercent() returned %v, and the contract says this reading has no usable percentage. Forwarding it is the whole defect: the number is real history and a false present", used)
			case c.UsedPercent != nil && !haveUsed:
				t.Errorf("UsedPercent() has no reading and the contract expects %v", *c.UsedPercent)
			case c.UsedPercent != nil && used != *c.UsedPercent:
				t.Errorf("UsedPercent() = %v, want %v", used, *c.UsedPercent)
			}
			if remaining, ok := got.RemainingPercent(); ok != haveUsed {
				t.Errorf("RemainingPercent() availability (%v) disagrees with UsedPercent() (%v) — a capacity derived from a percentage nobody has", ok, haveUsed)
			} else if ok && remaining != 100-used {
				t.Errorf("RemainingPercent() = %v, want %v", remaining, 100-used)
			}

			ttr, haveTTR := got.TimeToReset()
			switch {
			case c.SecondsToReset == nil && haveTTR:
				t.Errorf("TimeToReset() returned %v on a reading the contract says has none. The routing spec's `time_to_reset < 90 minutes` arm is trivially true on a negative number, which is exactly how a dead window produces a phantom SPEND_DOWN", ttr)
			case c.SecondsToReset != nil && !haveTTR:
				t.Errorf("TimeToReset() has no answer and the contract expects %ds", *c.SecondsToReset)
			case c.SecondsToReset != nil && int64(ttr/time.Second) != *c.SecondsToReset:
				t.Errorf("TimeToReset() = %v (%ds), want %ds", ttr, int64(ttr/time.Second), *c.SecondsToReset)
			}
			if haveTTR && ttr <= 0 {
				t.Errorf("TimeToReset() = %v — a current window's remaining time is strictly positive by construction; a non-positive one means the currency gate let a closed window through", ttr)
			}
			if _, ok := got.ResetsAt(); ok != haveTTR {
				t.Errorf("ResetsAt() availability (%v) disagrees with TimeToReset() (%v)", ok, haveTTR)
			}
		})
	}

	if err := tally.RequireEvery("the window-currency sweep", currencyCaseFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

// TestTheWireIsCurrentHintIsNeverTheDecision is the falsifiability check for
// the one sentence this guard exists for.
//
// A reader that consulted `is_current` would pass every case above, because the
// daemon's own answer agrees with the right one on all but one of them — it was
// computed correctly, at generated_at. Flipping the hint on every case and
// requiring the verdicts not to move is what makes "display-only" checkable
// rather than a comment.
func TestTheWireIsCurrentHintIsNeverTheDecision(t *testing.T) {
	fx := loadCurrencyFixture(t)
	flipped := 0
	for _, c := range fx.Cases {
		if c.Window == nil {
			continue
		}
		now := time.Unix(c.Now, 0)
		before := ReadWindow(c.Window, Provenance{}, now)

		mutant := *c.Window
		switch {
		case mutant.IsCurrent == nil:
			v := true
			mutant.IsCurrent = &v
		default:
			v := !*mutant.IsCurrent
			mutant.IsCurrent = &v
		}
		flipped++

		after := ReadWindow(&mutant, Provenance{}, now)
		if after.State != before.State || after.Reason != before.Reason {
			t.Errorf("%s: flipping the wire's is_current changed the verdict from %s/%s to %s/%s — the hint was computed at generated_at and is stale the moment the document is cached, which is exactly what a routing engine does to it",
				c.Name, before.State, before.Reason, after.State, after.Reason)
		}
	}
	if flipped < currencyCaseFloor-1 {
		t.Fatalf("only %d cases carried a window to mutate — this guard is running over almost nothing", flipped)
	}
}

// TestTheReasonVocabularyIsClosedBothWays holds the constants this package can
// produce to the reasons the corpus declares. One direction alone is not
// enough: "every fixture reason is a real constant" passes a package that has
// quietly grown a fourth reason no other reader has heard of.
func TestTheReasonVocabularyIsClosedBothWays(t *testing.T) {
	fx := loadCurrencyFixture(t)

	declared := map[string]bool{}
	for r := range fx.Vocabulary.RolledOverReasons {
		declared[r] = true
	}
	for r := range fx.Vocabulary.UnreadableReasons {
		declared[r] = true
	}
	if len(declared) == 0 {
		t.Fatal("the corpus declares no reasons at all — the vocabulary blocks were renamed and this guard is comparing nothing")
	}

	produced := map[string]bool{
		ReasonResetHasPassed:  true,
		ReasonResetEqualsNow:  true,
		ReasonNoResetTime:     true,
		ReasonNoWindowReading: true,
	}
	for r := range declared {
		if !produced[r] {
			t.Errorf("the corpus declares the reason %q and window.go can never produce it — a case pinned against an outcome that does not exist", r)
		}
	}
	for r := range produced {
		if !declared[r] {
			t.Errorf("window.go can answer %q and the corpus does not declare it — an unexercised classification arm is one nothing holds the copies to", r)
		}
	}

	// And the split is normative: a rolled-over reason must not be reachable
	// from an unreadable verdict or the two states blur.
	var rolled, unread []string
	for r := range fx.Vocabulary.RolledOverReasons {
		rolled = append(rolled, r)
	}
	for r := range fx.Vocabulary.UnreadableReasons {
		if fx.Vocabulary.RolledOverReasons[r] != "" {
			unread = append(unread, r)
		}
	}
	sort.Strings(rolled)
	if len(unread) > 0 {
		t.Errorf("%v are declared under BOTH verdicts — 'the window closed' and 'no window can be identified' are different answers and a shared reason erases the difference", unread)
	}
	if len(rolled) == 0 {
		t.Error("no rolled-over reasons declared")
	}
}

// TestReadWindowLeaksNothingOffAClosedWindow is the direct statement of the
// invariant, independent of the corpus: whatever a dead reading carries, every
// accessor refuses it.
func TestReadWindowLeaksNothingOffAClosedWindow(t *testing.T) {
	now := time.Unix(1788126404, 0)
	pct := 67.0
	past := now.Add(-47 * time.Hour).Unix()
	mins := int64(300)
	yes := true

	dead := ReadWindow(&WireWindow{
		UsedPercent:   &Measured{State: MeasuredOk, Value: &pct},
		ResetsAt:      &past,
		WindowMinutes: &mins,
		IsCurrent:     &yes, // the stale hint, as loud as it gets
	}, Provenance{Source: "disk"}, now)

	if dead.State != WindowRolledOver {
		t.Fatalf("state = %q, want rolled-over", dead.State)
	}
	if v, ok := dead.UsedPercent(); ok {
		t.Errorf("UsedPercent() = %v on a closed window", v)
	}
	if v, ok := dead.RemainingPercent(); ok {
		t.Errorf("RemainingPercent() = %v on a closed window", v)
	}
	if v, ok := dead.ResetsAt(); ok {
		t.Errorf("ResetsAt() = %v on a closed window", v)
	}
	if v, ok := dead.TimeToReset(); ok {
		t.Errorf("TimeToReset() = %v on a closed window — negative, and less than 90 minutes on any unguarded comparison", v)
	}
	if v, ok := dead.WindowLength(); ok {
		t.Errorf("WindowLength() = %v on a closed window", v)
	}
	if dead.Explain() == "" {
		t.Error("a decision that drops a provider must be able to say why")
	}
}
