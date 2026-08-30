package limits

// The demand forecast: the routing spec's §15, at the size §15 asks for.
//
//	"The first version does not need machine learning, because apparently we can
//	 resist that temptation once."
//
// So this is a weighted count and nothing more. The weights live in the routing
// matrix under `forecast_weights:` (scouting 1, implementation 4, review 2,
// fixing 2, validation 1) and arrive here as an argument; there is no phase
// vocabulary and no number in this file.
//
// THE HONEST LIMIT, STATED RATHER THAN PAPERED OVER. The mode rules the
// forecast feeds (§14) compare demand against REMAINING CAPACITY, which means a
// percentage of an allowance. Weighted work units are not that, and turning one
// into the other needs a cost model — how much of a five-hour window one
// implementation actually consumes — which this repo does not have and will not
// have until the decision log from a later session has run for a while. So:
//
//   - A caller that KNOWS the share (it has been watching the meter, or it is a
//     test pinning a threshold) supplies it directly, and it is used.
//   - A caller that supplies expected WORK gets it weighted, and the units are
//     reported and explained — but they do NOT become a percentage, and the
//     forecast stays UNKNOWN for the purpose of the mode arms.
//   - A caller that supplies nothing gets UNKNOWN.
//
// An UNKNOWN forecast cannot satisfy the spend-down rule (all three arms must
// hold) and cannot satisfy the conserve rule's demand arm. That is the
// conservative direction on both counts: no promotion, and no phantom
// conservation. Inventing a conversion factor to make the arms fire would be
// exactly the failure this whole layer exists to avoid — a number that looks
// like evidence and is not.

import (
	"fmt"
	"sort"
)

// Work is a count of expected agent runs in one workflow phase before the
// window resets. The phase names are the matrix's `forecast_weights:` keys.
type Work struct {
	Phase string `json:"phase"`
	Count int    `json:"count"`
}

// Demand is the forecast.
type Demand struct {
	// PctOfAllowance is how much of the provider's allowance the work ahead is
	// expected to consume, as a percentage of the whole allowance. Meaningful
	// only when Known.
	//
	// NOT omitempty: 0 is the value that makes the spend-down arm fire, and a
	// field that vanishes at exactly the number a reader most needs to see is a
	// decision that cannot explain itself. `Known` is what says whether it means
	// anything.
	PctOfAllowance float64 `json:"pctOfAllowance"`
	// Known is whether anybody could actually say. False is a real answer and
	// it is the one the mode rules must handle, not a zero to compare against.
	Known bool `json:"known"`
	// Units is the §15 weighted work count, when the caller described work.
	Units float64 `json:"units,omitempty"`
	// Phases lists what was counted, weighted, in a stable order — so a
	// decision can show its arithmetic.
	Phases []string `json:"phases,omitempty"`
	// UnweightedPhases are phases the matrix has no weight for. They are
	// REPORTED rather than counted as zero: a phase nobody assigned a weight to
	// is a gap in the matrix, and silently treating it as free demand is how a
	// forecast talks itself into spend-down.
	UnweightedPhases []string `json:"unweightedPhases,omitempty"`
	// Because is the sentence a decision quotes.
	Because string `json:"because"`
}

// DemandUnknown is the honest answer, with the reason attached.
func DemandUnknown(because string) Demand {
	return Demand{Known: false, Because: because}
}

// DemandFromPercent is a share the caller measured or asserted.
//
// Negative is refused rather than clamped: a negative demand would make the
// spend-down arm trivially true, which is the same shape of bug as a negative
// time-to-reset, and clamping it to zero would hide the caller's mistake behind
// the most aggressive possible answer.
func DemandFromPercent(pct float64) Demand {
	if pct < 0 {
		return DemandUnknown(fmt.Sprintf("the caller forecast %.1f%% of the allowance, which is not a quantity of work", pct))
	}
	return Demand{
		PctOfAllowance: pct,
		Known:          true,
		Because:        fmt.Sprintf("the caller forecasts %.0f%% of the allowance will be spent before the reset", pct),
	}
}

// DemandFromWork weights expected work with the matrix's forecast weights.
//
// It returns UNITS, deliberately not a percentage — see the file header. The
// units are still worth computing: they are the §15 shape, they are what a
// later session's decision log will calibrate against real consumption, and
// they let a decision say "four implementations and two reviews are queued"
// rather than "demand unknown" with no detail at all.
func DemandFromWork(work []Work, weights map[string]float64) Demand {
	d := Demand{Known: false}
	counted := map[string]float64{}
	for _, w := range work {
		if w.Count <= 0 {
			continue
		}
		weight, ok := weights[w.Phase]
		if !ok {
			d.UnweightedPhases = append(d.UnweightedPhases, w.Phase)
			continue
		}
		d.Units += weight * float64(w.Count)
		counted[w.Phase] += float64(w.Count)
	}
	for phase, n := range counted {
		d.Phases = append(d.Phases, fmt.Sprintf("%s x%.0f @ %g", phase, n, weights[phase]))
	}
	sort.Strings(d.Phases)
	sort.Strings(d.UnweightedPhases)

	switch {
	case len(work) == 0:
		d.Because = "no expected work was described, so demand before the reset is unknown"
	case d.Units == 0 && len(d.UnweightedPhases) > 0:
		d.Because = fmt.Sprintf("the work ahead is entirely in phases the matrix has no forecast_weights for (%v), so demand is unknown", d.UnweightedPhases)
	default:
		d.Because = fmt.Sprintf("%g weighted work unit(s) ahead (%v) — the matrix has no cost model to turn work units into a share of an allowance, so demand stays unknown", d.Units, d.Phases)
	}
	return d
}

// Forecast is the entry point the routing layer calls: the caller's own share
// when it gave one, otherwise the weighted work count, otherwise unknown.
//
// pct is nil when the caller supplied no share at all. That distinction is the
// whole reason it is a pointer: `0` is a REAL forecast ("nothing more is coming
// before the reset, spend it down") and is exactly the value that makes the
// spend-down arm fire, so it must never be reachable by omission.
func Forecast(pct *float64, work []Work, weights map[string]float64) Demand {
	if pct != nil {
		return DemandFromPercent(*pct)
	}
	return DemandFromWork(work, weights)
}
