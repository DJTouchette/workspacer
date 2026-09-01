package routing

// THE PACING BLOCK, from the file to the arithmetic.
//
// routing.yaml carries `thresholds.pacing:`; internal/limits carries the
// division. This file is the only bridge between them, and it exists so that
// every knob in that block has exactly one reader and it is named.
//
// The rule this file exists to keep is the one the fleet gets wrong most often:
// A SETTING THAT IS WRITTEN AND NEVER READ LOOKS EXACTLY LIKE A WORKING ONE.
// So the mapping below is total — every field of Pacing appears in PaceConfig,
// and TestEveryPacingKnobReachesTheArithmetic in pacing_test.go fails if one
// stops.
//
// Timezone resolution lives here rather than in internal/limits because a
// location is looked UP (from the host's tzdata) rather than computed, and
// limits.PaceConfig takes an already-resolved *time.Location so its arithmetic
// stays pure and testable against a fixed zone.

import (
	"fmt"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// TimezoneLocal is the `timezone:` value meaning "whatever zone this host is
// in". It is the shipped default: a fleet's weekend is its operator's weekend,
// and the hub runs on the operator's machine.
const TimezoneLocal = "local"

// PaceConfig turns the matrix's pacing block into the arithmetic's inputs.
//
// A matrix that cannot answer (nil) yields a DISABLED config rather than a
// zero-valued enabled one: no matrix means no bands, and a pace verdict taken
// against bands nobody wrote would be a number invented by a struct literal.
func (m *Matrix) PaceConfig() limits.PaceConfig {
	if m == nil {
		return limits.PaceConfig{}
	}
	p := m.Thresholds.Pacing
	loc, _ := resolveTimezone(p.SevenDay.Timezone)
	return limits.PaceConfig{
		Enabled:               p.IsEnabled(),
		ConserveAtRatio:       p.ConserveAtRatio,
		BlockSpendDownAtRatio: p.BlockSpendDownAtRatio,
		MinElapsedPct:         p.Bootstrap.MinElapsedPct,
		ExpectedOffsetPct:     p.Bootstrap.ExpectedOffsetPct,
		Curve:                 normalizePacingWord(p.SevenDay.Curve),
		Location:              loc,
		WeekendWeight:         p.SevenDay.WeekendWeight,
		WeekendPolicy:         normalizePacingWord(p.SevenDay.Weekend),
		WeekendReservePct:     p.SevenDay.WeekendReservePct,
	}
}

// resolveTimezone maps the file's `timezone:` onto a real location.
//
// `local` and an empty value both mean the host's zone. Anything else is an
// IANA name, and one that this host's tzdata cannot answer for yields a NIL
// location — which limits.PaceConfig.UsableCurve turns into a calendar-curve
// fallback with the reason stated, rather than silently computing a weekend in
// UTC for a fleet thirteen hours away from it.
func resolveTimezone(name string) (*time.Location, error) {
	trimmed := strings.TrimSpace(name)
	if trimmed == "" || strings.EqualFold(trimmed, TimezoneLocal) {
		return time.Local, nil
	}
	loc, err := time.LoadLocation(trimmed)
	if err != nil {
		return nil, err
	}
	return loc, nil
}

func normalizePacingWord(v string) string {
	return strings.ToLower(strings.TrimSpace(v))
}

// validatePacing is the load-time half, and it reports rather than refuses —
// the same policy as every other Issue in this package. A pacing block that
// cannot order anything makes pace UNKNOWN at decision time, which conserves
// nothing and unlocks nothing; the Issue is how an operator finds out at save
// time instead of by wondering why nothing ever paces.
func validatePacing(p Pacing) []Issue {
	var issues []Issue
	add := func(where, format string, args ...any) {
		issues = append(issues, Issue{Where: where, Detail: fmt.Sprintf(format, args...)})
	}
	const at = "thresholds.pacing"

	if !p.IsEnabled() {
		// Everything below describes how pacing behaves, and it does not.
		// Reporting a band as wrong in a file that has pacing switched off
		// would be noise, and the OFF state is a legitimate configuration —
		// it is the one that reproduces the pre-pacing answers exactly.
		return nil
	}

	switch {
	case p.ConserveAtRatio <= 0:
		add(at, "conserve_at_ratio is %g — a pace band must be a positive ratio of consumed-to-expected, so no pace verdict will ever be taken", p.ConserveAtRatio)
	case p.ConserveAtRatio < 1:
		add(at, "conserve_at_ratio is %g, which is BELOW 1.0: spending exactly on the curve would be treated as overspending, so this fleet would conserve permanently", p.ConserveAtRatio)
	}
	switch {
	case p.BlockSpendDownAtRatio <= 0:
		add(at, "block_spend_down_at_ratio is %g — a pace band must be a positive ratio, so pace can never block a spend-down", p.BlockSpendDownAtRatio)
	case p.BlockSpendDownAtRatio > p.ConserveAtRatio && p.ConserveAtRatio > 0:
		add(at, "block_spend_down_at_ratio %g is ABOVE conserve_at_ratio %g, so there is a band that conserves without blocking spend-down — a mode cannot be both", p.BlockSpendDownAtRatio, p.ConserveAtRatio)
	}

	if p.Bootstrap.MinElapsedPct <= 0 {
		add(at+".bootstrap", "min_elapsed_pct is %g, so a window is paced from its first second — one percent used against a fifth of a percent elapsed is a ratio of five, and every window would open in CONSERVE", p.Bootstrap.MinElapsedPct)
	}
	if p.Bootstrap.MinElapsedPct >= 100 {
		add(at+".bootstrap", "min_elapsed_pct is %g, which no window ever reaches, so pace is never judged at all", p.Bootstrap.MinElapsedPct)
	}
	if p.Bootstrap.ExpectedOffsetPct < 0 {
		add(at+".bootstrap", "expected_offset_pct is %g — a negative offset NARROWS the denominator, which is the opposite of what the bootstrap guard is for", p.Bootstrap.ExpectedOffsetPct)
	}
	if p.Bootstrap.ExpectedOffsetPct >= 100 {
		add(at+".bootstrap", "expected_offset_pct is %g, which pins the expected share at the whole allowance, so nothing can ever be over the curve", p.Bootstrap.ExpectedOffsetPct)
	}

	sd := p.SevenDay
	curve := normalizePacingWord(sd.Curve)
	switch curve {
	case limits.CurveCalendar, limits.CurveWorkdays:
	case "":
		add(at+".seven_day", "no curve named — write `curve: calendar` or `curve: workdays`")
	default:
		add(at+".seven_day", "curve %q is not a seven-day curve (%s, %s), so the calendar curve answers", sd.Curve, limits.CurveCalendar, limits.CurveWorkdays)
	}
	if _, err := resolveTimezone(sd.Timezone); err != nil {
		add(at+".seven_day", "timezone %q cannot be resolved on this host (%v), so the workdays curve has no day boundaries and the calendar curve answers instead", sd.Timezone, err)
	}
	if curve == limits.CurveWorkdays && !(sd.WeekendWeight > 0) {
		add(at+".seven_day", "weekend_weight is %g and the workdays curve is selected — a zero or negative weekend weight leaves a window that ends over a weekend with no expected progress at all, so every weekend hour would read as infinite overspend. Use a small positive weight (0.25–0.5); the curve falls back to `calendar` until you do", sd.WeekendWeight)
	}
	if sd.WeekendWeight > 1 {
		add(at+".seven_day", "weekend_weight is %g, which budgets a weekend hour MORE than a weekday one — legal, and almost certainly not what was meant", sd.WeekendWeight)
	}
	switch normalizePacingWord(sd.Weekend) {
	case limits.WeekendSpendTail:
		if sd.WeekendReservePct > 0 {
			add(at+".seven_day", "weekend_reserve_pct is %g but weekend is `%s`, which holds nothing back — the reserve is IGNORED. Write `weekend: %s` to make it bite", sd.WeekendReservePct, limits.WeekendSpendTail, limits.WeekendReserve)
		}
	case limits.WeekendReserve:
		if sd.WeekendReservePct <= 0 || sd.WeekendReservePct >= 100 {
			add(at+".seven_day", "weekend is `%s` but weekend_reserve_pct is %g, which is not a share between 0 and 100 — nothing is reserved and the curve is unchanged", limits.WeekendReserve, sd.WeekendReservePct)
		}
	case "":
		add(at+".seven_day", "no weekend policy named — write `weekend: %s` or `weekend: %s`", limits.WeekendSpendTail, limits.WeekendReserve)
	default:
		add(at+".seven_day", "weekend %q is not a weekend policy (%s, %s), so nothing is held back", sd.Weekend, limits.WeekendSpendTail, limits.WeekendReserve)
	}
	return issues
}
