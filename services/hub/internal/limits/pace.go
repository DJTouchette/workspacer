package limits

// PACE: are we spending this window faster than it refills?
//
// Health (health.go) answers "how much is gone". It cannot answer "is that a
// lot for a Tuesday morning", and those are different questions with different
// answers: 40% of a seven-day window is comfortable on Friday and is a fleet
// about to run dry on Monday. The missing term was always HOW FAR THROUGH THE
// WINDOW THE READING IS, which needs the window's LENGTH — a fact claudemon now
// carries for the two Anthropic windows as well as the two Codex ones.
//
// THE ARITHMETIC IS ONE DIVISION AND THE GUARDS ARE THE FILE.
//
//	elapsed  = (length - time_to_reset) / length      how far through we are
//	expected = the curve's share of the allowance that SHOULD be gone by now
//	ratio    = (used_percent / 100) / expected        > 1 is overspending
//
// Every input above is read through Reading, which refuses a percentage, a
// reset or a length off a window that has closed. There is no arm in this file
// that can see the stale 67% the currency guard exists for, and there is no
// second path to a duration — see window.go's ReadWindow.
//
// THREE WAYS THIS COULD LIE, AND WHAT STOPS EACH:
//
//  1. THE START OF A WINDOW. One percent used against 0.2% elapsed is a ratio of
//     five, and reading it as "overspending" would put a fleet into conserve for
//     the first ten minutes of every five-hour window. The bootstrap floor
//     refuses to judge at all below a configured elapsed share, and the offset
//     widens the denominator so an early burst does not divide by nearly zero.
//     Both are configuration (routing.yaml `thresholds.pacing.bootstrap`),
//     because the right numbers are a matter of taste about a fleet's shape.
//
//  2. THE WEEKEND. A seven-day window spent against a five-day working week is
//     not overspending on Thursday — it is on plan. The workdays curve weights
//     weekend hours down so the expected curve tracks when the work actually
//     happens, and the weight must be NONZERO: at zero, a window that ends over
//     a weekend has a zero denominator and every weekend hour reads as infinite
//     overspend. The curve is OPT-IN (`curve: calendar` ships) because a fleet
//     that runs at the weekend would be told to conserve on Saturday for no
//     reason.
//
//  3. A WINDOW NOBODY MEASURED. Copilot publishes nothing, an Anthropic monthly
//     overage window has no fixed length, and a provider absent from the report
//     has no window at all. Every one of those is PaceUnknown, which conserves
//     nothing and unlocks nothing. An invented denominator would be exactly the
//     "number that looks like evidence and is not" this layer exists to refuse.
//
// WHAT THIS PACKAGE STILL DOES NOT DO: decide. A ratio is a fact; whether a
// ratio of 1.4 should conserve is internal/routing's question, answered from
// routing.yaml's bands. Nothing here reads a threshold it was not handed.

import (
	"fmt"
	"math"
	"sort"
	"time"
)

// The seven-day curves. `calendar` is linear in wall-clock time; `workdays`
// weights weekend hours by PaceConfig.WeekendWeight.
const (
	CurveCalendar = "calendar"
	CurveWorkdays = "workdays"
)

// What the weekend is FOR, under the workdays curve.
const (
	// WeekendSpendTail lets the tail of the allowance be spent at the weekend:
	// the curve reaches 100% at the reset and nothing is held back.
	WeekendSpendTail = "spend_tail"
	// WeekendReserve holds a share of the allowance back against the curve, so
	// the pacer starts complaining earlier during the week.
	WeekendReserve = "reserve"
)

// maxCurveDays bounds the day walk in weightedSeconds. A seven-day window takes
// eight steps; anything claiming to be months long is a window this curve was
// not designed for and is answered UNKNOWN rather than walked.
const maxCurveDays = 62

// PaceConfig is the pacing policy, supplied by the caller. Like Bands, every
// number in it comes from routing.yaml (`thresholds.pacing`) and none of them
// is written here.
type PaceConfig struct {
	// Enabled false makes every reading PaceDisabled, which is how the whole
	// feature is switched off without changing a single other input.
	Enabled bool
	// ConserveAtRatio is where a ratio starts meaning "conserve".
	ConserveAtRatio float64
	// BlockSpendDownAtRatio is where a ratio stops licensing spend-down. It is
	// separate from, and never above, ConserveAtRatio: being slightly ahead of
	// the curve is a reason not to spend the remainder early, and is not yet a
	// reason to economize.
	BlockSpendDownAtRatio float64

	// MinElapsedPct is the bootstrap FLOOR: below this share of the window
	// elapsed, no pace verdict is produced at all.
	MinElapsedPct float64
	// ExpectedOffsetPct is the bootstrap OFFSET, in percentage points added to
	// the expected share before the division.
	ExpectedOffsetPct float64

	// Curve is which seven-day curve to use; it applies to the seven_day window
	// only, because a five-hour window has no weekday shape.
	Curve string
	// Location is the timezone whose day boundaries the workdays curve uses.
	// nil means the workdays curve cannot be evaluated and the calendar curve
	// answers instead — a weekend is a local fact, and a curve computed in UTC
	// for a fleet in UTC+13 is wrong by most of a day.
	Location *time.Location
	// WeekendWeight is one weekend hour's share of one weekday hour's budget.
	// MUST be strictly positive; see the file header.
	WeekendWeight float64
	// WeekendPolicy is spend_tail or reserve.
	WeekendPolicy string
	// WeekendReservePct is the share of the whole allowance held back under the
	// reserve policy. Ignored (and said so) under spend_tail.
	WeekendReservePct float64
}

// UsableCurve reports whether the workdays curve can actually be evaluated, and
// why not when it cannot.
//
// It is separate from the load-time validation on purpose: a matrix issue is
// reported once when the file is read, and this is what stops a decision made
// against a bad value from dividing by zero an hour later. A refused curve falls
// back to the calendar one, which is always evaluable, and the fallback is
// stated in the explanation rather than being silent.
func (c PaceConfig) UsableCurve() (workdays bool, why string) {
	if c.Curve != CurveWorkdays {
		return false, ""
	}
	if c.Location == nil {
		return false, "no timezone could be resolved, and a weekend is a local fact — using the calendar curve"
	}
	if !(c.WeekendWeight > 0) {
		return false, fmt.Sprintf(
			"weekend_weight is %g, which is not a usable weight (a zero-weight weekend makes a window that ends over one have no expected progress at all, so every weekend hour would read as infinite overspend) — using the calendar curve",
			c.WeekendWeight)
	}
	return true, ""
}

// Bands reports whether the ratio bands can order anything. Like Bands.Valid in
// health.go, an unusable pair answers UNKNOWN rather than inventing numbers.
func (c PaceConfig) BandsValid() bool {
	return c.ConserveAtRatio > 0 &&
		c.BlockSpendDownAtRatio > 0 &&
		c.BlockSpendDownAtRatio <= c.ConserveAtRatio
}

// PaceState is the verdict for one window.
type PaceState string

const (
	// PaceUnknown means no ratio could be computed — no window length, no
	// current reading, no utilization, or too early in the window to judge.
	// It conserves nothing and unlocks nothing.
	PaceUnknown PaceState = "unknown"
	// PaceDisabled means pacing is switched off in routing.yaml. Distinct from
	// unknown: nothing was missing, nobody asked.
	PaceDisabled PaceState = "disabled"
	// PaceOnTrack means consumption is at or below the curve.
	PaceOnTrack PaceState = "on_track"
	// PaceAhead means consumption is above the curve but below the conserve
	// band: not a reason to economize, and a reason not to spend the remainder
	// early.
	PaceAhead PaceState = "ahead"
	// PaceOverspending means consumption is above the conserve band.
	PaceOverspending PaceState = "overspending"
)

// PaceReport is one window's pace, with the arithmetic kept so a decision can
// show it. Nothing here is a decision.
type PaceReport struct {
	Window string    `json:"window"`
	State  PaceState `json:"state"`
	// Known is whether Ratio means anything. It is a separate field rather than
	// a nil Ratio because 0 is a real ratio (nothing used yet).
	Known bool `json:"known"`
	// Ratio is used share / expected share. Meaningful only when Known.
	Ratio float64 `json:"ratio"`
	// UsedPct / ExpectedPct / ElapsedPct are the terms of that division, as
	// percentages, so a reader can check it. Present only when Known.
	UsedPct     float64 `json:"usedPct,omitempty"`
	ExpectedPct float64 `json:"expectedPct,omitempty"`
	ElapsedPct  float64 `json:"elapsedPct,omitempty"`
	// Curve is the curve that actually produced ExpectedPct — which is not
	// always the configured one, since an unusable workdays curve falls back.
	Curve string `json:"curve,omitempty"`
	// Because is the sentence a routing decision quotes.
	Because string `json:"because"`
}

// Conserves reports whether this pace is a reason to conserve.
func (p PaceReport) Conserves() bool { return p.Known && p.State == PaceOverspending }

// BlocksSpendDown reports whether this pace is a reason NOT to spend the
// remainder of the window early. Overspending blocks it too, which is why this
// is not an equality test: a state that conserves and did not block spend-down
// would be a contradiction reachable by adding one band.
func (p PaceReport) BlocksSpendDown() bool {
	return p.Known && (p.State == PaceAhead || p.State == PaceOverspending)
}

// PaceFor judges ONE bucket's window.
//
// Every fact it reads comes off b.Reading. It never touches the ungated raw
// scalar, and it cannot: DisplayOnlyRawUsedPercent is not called here and the
// routing package's source scanner would catch it there.
func PaceFor(b Bucket, cfg PaceConfig) PaceReport {
	p := PaceReport{Window: b.Window, State: PaceUnknown}
	if !cfg.Enabled {
		p.State = PaceDisabled
		p.Because = "pacing is switched off in routing.yaml (thresholds.pacing.enabled: false), so window progress changes no answer"
		return p
	}
	if !cfg.BandsValid() {
		p.Because = fmt.Sprintf(
			"routing.yaml's thresholds.pacing bands cannot order anything (conserve_at_ratio %g, block_spend_down_at_ratio %g), so no pace verdict is taken",
			cfg.ConserveAtRatio, cfg.BlockSpendDownAtRatio)
		return p
	}
	if !b.Reading.Usable() {
		p.Because = fmt.Sprintf("%s pace unknown: %s", b.Window, b.Reading.Explain())
		return p
	}
	if b.Fresh != nil && !*b.Fresh {
		// The currency guard only refuses a window that has CLOSED. A window
		// can still be the one running — resets_at strictly in the future —
		// while the daemon's own reading of it is old: a 72-hour-old
		// observation of a still-current seven-day window paces exactly as
		// confidently as a reading taken a second ago unless something stops
		// it here. That under-conserves in the same direction the currency
		// guard exists to close, so a non-fresh reading gets the same
		// answer as a rolled-over one: PaceUnknown, which conserves nothing
		// and unlocks nothing, and WorstPace skips it exactly as it skips a
		// window nobody could read at all.
		p.Because = fmt.Sprintf(
			"%s pace unknown: the daemon flagged this reading stale (fresh: false)%s — the window is still current but its evidence is too old to divide by",
			b.Window, staleAgeClause(b))
		return p
	}
	length, ok := b.Reading.WindowLength()
	if !ok {
		p.Because = fmt.Sprintf(
			"%s pace unknown: the source reports no window length, so there is no denominator for how far through the window this reading is",
			b.Window)
		return p
	}
	ttr, ok := b.Reading.TimeToReset()
	if !ok || ttr > length {
		p.Because = fmt.Sprintf(
			"%s pace unknown: the reset is further away than the window is long (%s left of a %s window), so the two facts disagree and neither can be divided by the other",
			b.Window, ttr.Round(time.Minute), length.Round(time.Minute))
		return p
	}
	used, ok := b.Reading.UsedPercent()
	if !ok {
		p.Because = fmt.Sprintf(
			"%s pace unknown: the window is the one running, but its utilization is unreadable — currency and readability are separate axes",
			b.Window)
		return p
	}

	elapsed := float64(length-ttr) / float64(length)
	p.ElapsedPct = elapsed * 100
	if p.ElapsedPct < cfg.MinElapsedPct {
		p.Because = fmt.Sprintf(
			"%s pace not judged: only %.1f%% of the window has elapsed, under the %.0f%% bootstrap floor — a ratio taken this early divides by nearly nothing and would read a normal first burst as overspending",
			b.Window, p.ElapsedPct, cfg.MinElapsedPct)
		return p
	}

	resetsAt, ok := b.Reading.ResetsAt()
	if !ok {
		// Unreachable: TimeToReset above is derived from the same guard. Stated
		// rather than assumed, because the whole file's claim is that no number
		// here comes from an unchecked path.
		p.Because = fmt.Sprintf("%s pace unknown: this reading has no reset time", b.Window)
		return p
	}

	expected, curve, note := expectedShare(b.Window, resetsAt.Add(-length), b.Reading.Now(), resetsAt, elapsed, cfg)
	p.Curve = curve
	if expected <= 0 {
		p.Because = fmt.Sprintf("%s pace unknown: the %s curve puts no expected consumption at this point in the window", b.Window, curve)
		return p
	}
	p.ExpectedPct = expected * 100
	p.UsedPct = used
	p.Ratio = (used / 100) / expected
	p.Known = true

	switch {
	case p.Ratio >= cfg.ConserveAtRatio:
		p.State = PaceOverspending
	case p.Ratio >= cfg.BlockSpendDownAtRatio:
		p.State = PaceAhead
	default:
		p.State = PaceOnTrack
	}
	p.Because = fmt.Sprintf(
		"%s is %.0f%% used at %.0f%% elapsed, against a %s curve expecting %.0f%% by now — pace %.2fx (%s)%s",
		b.Window, used, p.ElapsedPct, curve, p.ExpectedPct, p.Ratio, p.State, note)
	return p
}

// staleAgeClause names how old a stale reading's evidence is, when the
// account row carries an observed_at, so the pace explanation says more than
// just the word "stale".
func staleAgeClause(b Bucket) string {
	if b.ObservedAt == nil {
		return ""
	}
	age := b.Reading.Now().Sub(*b.ObservedAt)
	if age <= 0 {
		return ""
	}
	return fmt.Sprintf(" (observed %s ago)", age.Round(time.Minute))
}

// expectedShare is the curve: how much of the allowance SHOULD be gone by now.
//
// The calendar curve is elapsed time and nothing else, and it is what every
// window that is not the seven-day one gets — a five-hour window has no weekday
// shape, and a monthly overage window reports no length at all so it never
// arrives here.
func expectedShare(window string, start, now, end time.Time, elapsed float64, cfg PaceConfig) (share float64, curve string, note string) {
	curve, share = CurveCalendar, elapsed

	if window == WindowSevenDay {
		if workdays, why := cfg.UsableCurve(); workdays {
			total := weightedSeconds(start, end, cfg)
			done := weightedSeconds(start, now, cfg)
			if total > 0 && done >= 0 {
				curve, share = CurveWorkdays, done/total
			} else {
				note = "; the workdays curve produced no usable total, so the calendar curve answered"
			}
		} else if why != "" {
			note = "; " + why
		}
		share, note = applyWeekendReserve(share, note, cfg)
	}

	if cfg.ExpectedOffsetPct > 0 {
		share += cfg.ExpectedOffsetPct / 100
		note += fmt.Sprintf("; the bootstrap offset widened the expected share by %.0f points", cfg.ExpectedOffsetPct)
	}
	return math.Min(share, 1), curve, note
}

// applyWeekendReserve is the `weekend:` policy, and it is the one knob whose
// two settings are not two numbers.
//
//	spend_tail  nothing is held back: the curve reaches the whole allowance at
//	            the reset, so a fleet that is behind on Friday may use the
//	            remainder over the weekend without being told to conserve.
//	reserve     the curve is scaled to (100 - weekend_reserve_pct), so the pacer
//	            expects that share to still be there at the reset and starts
//	            saying "overspending" earlier during the week.
//
// A reserve of 0 under `reserve` is a no-op and is reported at load as such; a
// nonzero reserve under `spend_tail` is IGNORED, and says so here rather than
// being a number in a file that changes nothing.
func applyWeekendReserve(share float64, note string, cfg PaceConfig) (float64, string) {
	switch cfg.WeekendPolicy {
	case WeekendReserve:
		if cfg.WeekendReservePct <= 0 || cfg.WeekendReservePct >= 100 {
			return share, note + "; weekend: reserve holds nothing back because weekend_reserve_pct is not a share between 0 and 100"
		}
		scaled := share * (1 - cfg.WeekendReservePct/100)
		return scaled, note + fmt.Sprintf("; weekend: reserve holds %.0f%% of the allowance back, tightening the curve", cfg.WeekendReservePct)
	default:
		if cfg.WeekendReservePct > 0 {
			return share, note + fmt.Sprintf("; weekend_reserve_pct %.0f is IGNORED under weekend: spend_tail, which holds nothing back", cfg.WeekendReservePct)
		}
		return share, note
	}
}

// weightedSeconds integrates the day weight over [from, to) in the configured
// timezone: a weekday second counts 1, a weekend second counts WeekendWeight.
//
// Day boundaries come from time.Date in the location, so a DST day is 23 or 25
// hours long exactly as the calendar says, and a fleet's Saturday is its own
// Saturday rather than UTC's.
func weightedSeconds(from, to time.Time, cfg PaceConfig) float64 {
	if !to.After(from) || cfg.Location == nil {
		return 0
	}
	total := 0.0
	cur := from
	for i := 0; cur.Before(to); i++ {
		if i > maxCurveDays {
			// A window longer than this curve was designed for. Refusing is the
			// answer: an unbounded walk in a routing decision is worse than a
			// calendar fallback.
			return 0
		}
		local := cur.In(cfg.Location)
		next := time.Date(local.Year(), local.Month(), local.Day(), 0, 0, 0, 0, cfg.Location).AddDate(0, 0, 1)
		if !next.After(cur) {
			// A DST transition that lands exactly at local midnight (Chile,
			// Cuba, Paraguay, Lebanon and Azerbaijan all have or have had
			// one) makes the requested midnight a WALL-CLOCK TIME THAT DOES
			// NOT EXIST, and time.Date is then free to normalize it onto or
			// before `cur` instead of a day forward — live, requesting
			// America/Santiago 2026-09-06 00:00 (the spring-forward date)
			// returns 2026-09-05 23:00:00, not a day past `cur`. The day
			// boundary cannot be trusted at this instant, so fall through to
			// a flat 24h of REAL elapsed time instead: that is always
			// strictly positive regardless of what the local clock does,
			// so the walk always terminates, and the following iteration is
			// back on ordinary midnight-snapped footing since `cur` no
			// longer sits on the broken instant.
			next = cur.Add(24 * time.Hour)
		}
		if next.After(to) {
			next = to
		}
		w := 1.0
		switch local.Weekday() {
		case time.Saturday, time.Sunday:
			w = cfg.WeekendWeight
		}
		total += w * next.Sub(cur).Seconds()
		cur = next
	}
	return total
}

// PaceForAccount judges every window of one (provider, account) and folds them.
//
// THE FOLD IS THE WORST KNOWN RATIO, which is what makes an Anthropic decision
// "the worse of the five-hour and the seven-day pace" without either window
// being named in the policy: both are readable, so both are judged, and the one
// that binds is the one reported. Codex publishes a five-hour window that is
// frequently stale and a seven-day one that is not, so in practice the seven-day
// pace is what answers for it — by the currency guard rather than by a special
// case.
//
// A provider with no judgeable window folds to the FIRST unknown report, with
// its reason, rather than to a manufactured on-track: unknown conserves nothing
// and unlocks nothing, which is the conservative answer on both counts.
func PaceForAccount(buckets []Bucket, provider, account string, cfg PaceConfig) (worst PaceReport, all []PaceReport) {
	for _, b := range buckets {
		if b.Provider != provider || !b.AccountKnown || b.Account != account {
			continue
		}
		all = append(all, PaceFor(b, cfg))
	}
	sort.SliceStable(all, func(i, j int) bool {
		return windowRank(all[i].Window) < windowRank(all[j].Window)
	})
	return WorstPace(all), all
}

// WorstPace is the highest KNOWN ratio, or the first unknown when nothing is
// knowable. An unknown never outranks a known one: a window nobody could read
// must not be able to veto a window that was read.
func WorstPace(reports []PaceReport) PaceReport {
	var worst PaceReport
	found := false
	for _, r := range reports {
		if !r.Known {
			continue
		}
		if !found || r.Ratio > worst.Ratio {
			worst, found = r, true
		}
	}
	if found {
		return worst
	}
	if len(reports) > 0 {
		return reports[0]
	}
	return PaceReport{
		State:   PaceUnknown,
		Because: "no window of this provider could be paced at all, so window progress changes nothing here",
	}
}
