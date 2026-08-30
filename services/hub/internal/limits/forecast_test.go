package limits

import "testing"

// The weights the shipped matrix carries, as a fixture. The forecast must not
// have them as constants — they live in routing.yaml under forecast_weights:.
var testWeights = map[string]float64{
	"scouting": 1, "implementation": 4, "review": 2, "fixing": 2, "validation": 1,
}

func TestForecastDistinguishesZeroFromUnknown(t *testing.T) {
	zero := 0.0
	half := 50.0
	negative := -5.0

	for _, tc := range []struct {
		name      string
		pct       *float64
		work      []Work
		wantKnown bool
		wantPct   float64
		why       string
	}{
		{"a supplied zero is a real forecast", &zero, nil, true, 0,
			"'nothing more is coming before the reset' is the answer that UNLOCKS spend-down, so it must be reachable — and reachable only by saying it"},
		{"a supplied share is used", &half, nil, true, 50, ""},
		{"a supplied NEGATIVE share is refused, not clamped", &negative, nil, false, 0,
			"clamping to zero would hide the caller's mistake behind the most aggressive possible answer, which is the same shape as a negative time-to-reset passing a `< 90 minutes` arm"},
		{"omitting the field is UNKNOWN, not zero", nil, nil, false, 0,
			"THE DISTINCTION THE POINTER EXISTS FOR. A caller that says nothing must not be read as a caller that said 'nothing is coming'"},
		{"expected work weighs but does not become a percentage", nil,
			[]Work{{Phase: "implementation", Count: 2}, {Phase: "review", Count: 1}}, false, 0,
			"weighted units are not a share of an allowance, and there is no cost model to convert them. Inventing a factor to make the arms fire is the failure this layer exists to avoid"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := Forecast(tc.pct, tc.work, testWeights)
			if got.Known != tc.wantKnown {
				t.Errorf("Known = %v, want %v\n  why: %s", got.Known, tc.wantKnown, tc.why)
			}
			if got.Known && got.PctOfAllowance != tc.wantPct {
				t.Errorf("PctOfAllowance = %v, want %v", got.PctOfAllowance, tc.wantPct)
			}
			if got.Because == "" {
				t.Error("a forecast with no sentence cannot be quoted by a decision that rests on it")
			}
		})
	}
}

// TestWeightedWorkUsesTheMatrixWeights proves the `forecast_weights:` block
// actually reaches an arithmetic operation, rather than being a setting that is
// written and never read.
func TestWeightedWorkUsesTheMatrixWeights(t *testing.T) {
	got := DemandFromWork([]Work{
		{Phase: "implementation", Count: 2}, // 4 each
		{Phase: "review", Count: 3},         // 2 each
		{Phase: "scouting", Count: 1},       // 1 each
		{Phase: "nothing-like-this", Count: 9},
		{Phase: "review", Count: 0}, // a zero count contributes nothing
	}, testWeights)

	if got.Units != 15 {
		t.Errorf("Units = %v, want 15 (2*4 + 3*2 + 1*1) — the matrix's weights are not reaching the sum", got.Units)
	}
	if len(got.UnweightedPhases) != 1 || got.UnweightedPhases[0] != "nothing-like-this" {
		t.Errorf("UnweightedPhases = %v, want [nothing-like-this] — a phase nobody weighted must be REPORTED, because counting it as zero is how a forecast talks itself into spend-down", got.UnweightedPhases)
	}
	if got.Known {
		t.Error("weighted units became a KNOWN percentage — there is no cost model that could have produced one")
	}

	// A halved weight must halve the answer, or the map is being ignored and the
	// numbers happen to agree.
	halved := map[string]float64{"implementation": 2}
	if h := DemandFromWork([]Work{{Phase: "implementation", Count: 2}}, halved); h.Units != 4 {
		t.Errorf("with implementation weighted 2, two of them = %v units, want 4", h.Units)
	}
}
