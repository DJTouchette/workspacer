package limits

import (
	"math"
	"strings"
	"testing"
	"time"
)

// TestTheFiveDayCurveIsFlatAcrossTheWeekend is the whole claim of the "Work
// week (Monday–Friday)" schedule, stated as arithmetic rather than as a
// description: expected progress advances on weekdays and DOES NOT MOVE between
// Friday midnight and Monday midnight.
func TestTheFiveDayCurveIsFlatAcrossTheWeekend(t *testing.T) {
	// A seven-day window that opened 00:00 Monday and resets 00:00 the
	// following Monday, so the weekend sits inside it rather than at an edge.
	start := time.Date(2026, 8, 31, 0, 0, 0, 0, time.UTC) // Monday
	reset := start.AddDate(0, 0, 7)
	if start.Weekday() != time.Monday {
		t.Fatalf("fixture drift: start is %s", start.Weekday())
	}

	cfg := testPace()
	cfg.Curve = CurveFiveDay

	expectedAt := func(now time.Time) PaceReport {
		t.Helper()
		b := paceBucket("claude", WindowSevenDay, paceWin(50, reset.Unix(), 7*24*60), now)
		p := PaceFor(b, cfg)
		if !p.Known {
			t.Fatalf("%s pace unknown: %s", now.Weekday(), p.Because)
		}
		if p.Curve != CurveFiveDay {
			t.Fatalf("%s answered with the %q curve, not %q: %s", now.Weekday(), p.Curve, CurveFiveDay, p.Because)
		}
		return p
	}

	friMid := expectedAt(time.Date(2026, 9, 4, 12, 0, 0, 0, time.UTC)) // Friday noon
	friEnd := expectedAt(time.Date(2026, 9, 5, 0, 0, 0, 0, time.UTC))  // the instant the working week ends
	sat := expectedAt(time.Date(2026, 9, 5, 12, 0, 0, 0, time.UTC))    // Saturday noon
	sun := expectedAt(time.Date(2026, 9, 6, 23, 0, 0, 0, time.UTC))    // Sunday night
	if !(friMid.ExpectedPct < friEnd.ExpectedPct) {
		t.Errorf("the curve must still climb DURING Friday: noon %.3f%%, end of day %.3f%%", friMid.ExpectedPct, friEnd.ExpectedPct)
	}
	// The last second the window is still the one running: at the reset itself
	// the reading has rolled over and refuses, which is the currency guard's
	// job rather than the curve's.
	lastSecond := expectedAt(time.Date(2026, 9, 6, 23, 59, 59, 0, time.UTC))

	// FLAT: the weekend adds nothing at all.
	for _, tc := range []struct {
		what string
		got  float64
	}{{"saturday", sat.ExpectedPct}, {"sunday", sun.ExpectedPct}, {"the last second before the reset", lastSecond.ExpectedPct}} {
		if math.Abs(tc.got-friEnd.ExpectedPct) > 0.001 {
			t.Errorf("%s expects %.4f%% against Friday's %.4f%% — the five-day curve must not move over the weekend",
				tc.what, tc.got, friEnd.ExpectedPct)
		}
	}
	// And it reaches the WHOLE allowance by the end of the working week: the
	// five weekdays are the entire denominator.
	if math.Abs(friEnd.ExpectedPct-100) > 0.05 {
		t.Errorf("the five-day curve must expect the whole allowance to be gone by the end of Friday, got %.3f%%", friEnd.ExpectedPct)
	}

	// The weekend is flat for EXPECTED. Observed usage still counts: the same
	// instant with more used reads as further over the line.
	heavier := PaceFor(paceBucket("claude", WindowSevenDay, paceWin(90, reset.Unix(), 7*24*60),
		time.Date(2026, 9, 5, 12, 0, 0, 0, time.UTC)), cfg)
	if !heavier.Known || heavier.Ratio <= sat.Ratio {
		t.Errorf("weekend consumption must still move the ratio: 90%% used reads %.3fx against 50%%'s %.3fx (%s)",
			heavier.Ratio, sat.Ratio, heavier.Because)
	}
}

// TestTheFiveDayCurveMidweekAndAgainstItsNeighbours pins the two comparisons a
// reader needs to trust the word: five_day is STRICTLY ahead of both calendar
// and workdays mid-week (it packs the same allowance into fewer counted hours),
// and a window whose reset falls mid-week is weighted by real weekdays rather
// than by any Monday alignment.
func TestTheFiveDayCurveMidweekAndAgainstItsNeighbours(t *testing.T) {
	// Reset on a WEDNESDAY: the window runs Wed → Wed and contains its weekend
	// in the middle. Nothing in the curve may assume a Monday start.
	reset := time.Date(2026, 9, 9, 15, 0, 0, 0, time.UTC) // Wednesday 15:00
	if reset.Weekday() != time.Wednesday {
		t.Fatalf("fixture drift: reset is %s", reset.Weekday())
	}
	now := time.Date(2026, 9, 4, 12, 0, 0, 0, time.UTC) // Friday, mid-window
	b := paceBucket("claude", WindowSevenDay, paceWin(70, reset.Unix(), 7*24*60), now)

	cal := testPace()
	calendar := PaceFor(b, cal)

	work := cal
	work.Curve, work.WeekendWeight = CurveWorkdays, 0.5
	workdays := PaceFor(b, work)

	five := cal
	five.Curve = CurveFiveDay
	fiveDay := PaceFor(b, five)

	for _, p := range []PaceReport{calendar, workdays, fiveDay} {
		if !p.Known {
			t.Fatalf("%s pace unknown: %s", p.Curve, p.Because)
		}
	}
	if !(fiveDay.ExpectedPct > workdays.ExpectedPct && workdays.ExpectedPct > calendar.ExpectedPct) {
		t.Errorf("mid-week the curves must order five_day > workdays > calendar, got %.2f%% / %.2f%% / %.2f%%",
			fiveDay.ExpectedPct, workdays.ExpectedPct, calendar.ExpectedPct)
	}
	if fiveDay.Curve != CurveFiveDay {
		t.Errorf("the five_day curve did not answer: %q (%s)", fiveDay.Curve, fiveDay.Because)
	}
	// The explanation must name the curve that actually ran, or an operator
	// cannot tell which schedule produced the number.
	if !strings.Contains(fiveDay.Because, CurveFiveDay) {
		t.Errorf("the explanation never names the curve: %q", fiveDay.Because)
	}
}

// TestTheFiveDayCurveCannotDivideByZero is the zero-denominator case the
// "weekend_weight must be nonzero" rule exists for, exercised on the curve that
// deliberately sets it to zero.
func TestTheFiveDayCurveCannotDivideByZero(t *testing.T) {
	cfg := testPace()
	cfg.Curve = CurveFiveDay

	// The weight really is zero, and it comes from the CURVE rather than from
	// the matrix field (which still says 0.5 here).
	sat := time.Date(2026, 9, 5, 0, 0, 0, 0, time.UTC)
	if got := weightedSeconds(sat, sat.Add(12*time.Hour), cfg); got != 0 {
		t.Fatalf("a five-day weekend integrates to %v, not 0 — the curve is not weighting the weekend at zero", got)
	}

	// TOTAL is never zero for a real seven-day window, wherever it starts:
	// 7*24h always contains 120 weekday hours.
	for d := range 7 {
		start := time.Date(2026, 8, 31, 0, 0, 0, 0, time.UTC).AddDate(0, 0, d)
		if got := weightedSeconds(start, start.AddDate(0, 0, 7), cfg); math.Abs(got-120*3600) > 1 {
			t.Errorf("a seven-day window starting %s weighs %.0f weekday seconds, expected %.0f", start.Weekday(), got, 120*3600.0)
		}
	}

	// The one arm where DONE is zero: a window whose elapsed part is entirely
	// weekend. That must be UNKNOWN — no verdict — rather than an infinite
	// ratio, and it must say why.
	start := time.Date(2026, 9, 5, 0, 0, 0, 0, time.UTC) // Saturday 00:00
	now := time.Date(2026, 9, 6, 12, 0, 0, 0, time.UTC)  // Sunday noon
	b := paceBucket("claude", WindowSevenDay, paceWin(30, start.AddDate(0, 0, 7).Unix(), 7*24*60), now)
	got := PaceFor(b, cfg)
	if got.Known {
		t.Errorf("a weekend-only elapsed span produced a verdict (%.3fx): there is no expected weekday progress to divide by yet — %s", got.Ratio, got.Because)
	}
	if math.IsNaN(got.Ratio) || math.IsInf(got.Ratio, 0) {
		t.Errorf("ratio = %v — the zero-numerator arm produced a non-number", got.Ratio)
	}
	if !strings.Contains(got.Because, CurveFiveDay) {
		t.Errorf("the refusal must name the curve that refused: %q", got.Because)
	}
}

// TestTheFiveDayCurveRefusesAWeekendReserve is the "spend_tail must not distort
// this" requirement, held at the arithmetic: under five_day there is no weekend
// budget left for a reserve to hold back, so a reserve is ignored and says so.
func TestTheFiveDayCurveRefusesAWeekendReserve(t *testing.T) {
	start := time.Date(2026, 8, 31, 0, 0, 0, 0, time.UTC)
	now := time.Date(2026, 9, 3, 12, 0, 0, 0, time.UTC) // Thursday
	b := paceBucket("claude", WindowSevenDay, paceWin(60, start.AddDate(0, 0, 7).Unix(), 7*24*60), now)

	tail := testPace()
	tail.Curve = CurveFiveDay
	plain := PaceFor(b, tail)

	held := tail
	held.WeekendPolicy, held.WeekendReservePct = WeekendReserve, 20
	reserved := PaceFor(b, held)

	if math.Abs(reserved.ExpectedPct-plain.ExpectedPct) > 0.001 {
		t.Errorf("a weekend reserve moved the five-day curve (%.3f%% vs %.3f%%) — the schedule the user chose must not be scaled by a knob set for a different curve",
			reserved.ExpectedPct, plain.ExpectedPct)
	}
	if !strings.Contains(reserved.Because, "IGNORED") {
		t.Errorf("a reserve that changes nothing must SAY it changed nothing: %q", reserved.Because)
	}
	// And the weekend WEIGHT is not read either: the matrix value cannot move
	// a curve whose weekend is zero by definition.
	weighted := tail
	weighted.WeekendWeight = 0.9
	if got := PaceFor(b, weighted); math.Abs(got.ExpectedPct-plain.ExpectedPct) > 0.001 {
		t.Errorf("weekend_weight moved the five-day curve (%.3f%% vs %.3f%%)", got.ExpectedPct, plain.ExpectedPct)
	}
}

// TestTheFiveDayCurveUsesTheConfiguredZoneAndSurvivesDST is the two facts a
// weekday curve cannot get wrong: whose Saturday it is, and a day that is not
// 24 hours long.
func TestTheFiveDayCurveUsesTheConfiguredZoneAndSurvivesDST(t *testing.T) {
	// A fleet in UTC+13. 12:00 Saturday UTC is already Sunday there, and a
	// curve computed in UTC would be wrong by most of a day.
	auck, err := time.LoadLocation("Pacific/Auckland")
	if err != nil {
		t.Skipf("host tzdata has no Pacific/Auckland: %v", err)
	}
	start := time.Date(2026, 8, 31, 0, 0, 0, 0, auck) // local Monday
	reset := start.AddDate(0, 0, 7)
	// Local Friday 23:00 — the working week is over in Auckland, and it is
	// still Friday morning in UTC.
	now := time.Date(2026, 9, 4, 23, 0, 0, 0, auck)

	local := testPace()
	local.Curve, local.Location = CurveFiveDay, auck
	utc := local
	utc.Location = time.UTC

	b := paceBucket("claude", WindowSevenDay, paceWin(60, reset.Unix(), 7*24*60), now)
	inZone, inUTC := PaceFor(b, local), PaceFor(b, utc)
	if !inZone.Known || !inUTC.Known {
		t.Fatalf("unknown pace: local %s / utc %s", inZone.Because, inUTC.Because)
	}
	if math.Abs(inZone.ExpectedPct-inUTC.ExpectedPct) < 0.5 {
		t.Errorf("the configured zone made no difference (%.3f%% vs %.3f%%) — the weekend is being computed somewhere other than the fleet's own calendar",
			inZone.ExpectedPct, inUTC.ExpectedPct)
	}
	// No timezone at all falls back to the calendar curve and says so, rather
	// than guessing a weekend.
	none := local
	none.Location = nil
	if got := PaceFor(b, none); got.Curve != CurveCalendar || !strings.Contains(got.Because, "timezone") {
		t.Errorf("an unresolved timezone must fall back to the calendar curve with a reason, got %q: %s", got.Curve, got.Because)
	}

	// DST: Santiago springs forward AT local midnight, which is the transition
	// weightedSeconds has a named fallback for. The walk must terminate and the
	// weekday total must stay sane.
	santiago, err := time.LoadLocation("America/Santiago")
	if err != nil {
		t.Skipf("host tzdata has no America/Santiago: %v", err)
	}
	dst := testPace()
	dst.Curve, dst.Location = CurveFiveDay, santiago
	from := time.Date(2026, 9, 2, 0, 0, 0, 0, time.UTC)
	got := weightedSeconds(from, from.AddDate(0, 0, 7), dst)
	if got <= 0 || got > 7*24*3600 {
		t.Errorf("weightedSeconds across the Santiago spring-forward returned %.0f, which is not a sane weekday total for a seven-day window", got)
	}
}

// TestTheSchedulesLeaveTheShortWindowsIdentical is the promise that this
// setting is about the WEEK: whatever curve the seven-day window uses, the
// five-hour window's pace must be byte-for-byte the same, and the monthly
// overage window must stay unknown for want of a length.
func TestTheSchedulesLeaveTheShortWindowsIdentical(t *testing.T) {
	// A Saturday: the instant on which the two schedules disagree most about
	// the week, so an accidental leak into a short window would show here.
	now := time.Date(2026, 9, 5, 15, 0, 0, 0, time.UTC)
	if now.Weekday() != time.Saturday {
		t.Fatalf("fixture drift: %s", now.Weekday())
	}
	fiveHour := paceBucket("claude", WindowFiveHour, paceWin(72, now.Add(90*time.Minute).Unix(), 300), now)
	monthly := paceBucket("claude", WindowMonthly, paceWin(41, now.Add(72*time.Hour).Unix(), 0), now)

	seven := testPace()
	five := seven
	five.Curve = CurveFiveDay

	a, b := PaceFor(fiveHour, seven), PaceFor(fiveHour, five)
	if a != b {
		t.Errorf("the five-hour window differs between the two schedules:\n  seven_day: %+v\n  five_day:  %+v", a, b)
	}
	if !a.Known || a.Curve != CurveCalendar {
		t.Errorf("a five-hour window has no weekday shape and must pace on the calendar curve, got known=%v curve=%q (%s)", a.Known, a.Curve, a.Because)
	}
	if c, d := PaceFor(monthly, seven), PaceFor(monthly, five); c != d || c.Known {
		t.Errorf("the monthly overage window must be UNKNOWN under both schedules: %+v / %+v", c, d)
	}
}
