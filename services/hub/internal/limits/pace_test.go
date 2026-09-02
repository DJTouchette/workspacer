package limits

import (
	"math"
	"os"
	"strings"
	"testing"
	"time"
)

// readSource reads one of this package's own files, for the guards that are
// about what the SOURCE may contain rather than about what it computes.
func readSource(name string) (string, error) {
	raw, err := os.ReadFile(name)
	return string(raw), err
}

// testPace is the shipped matrix's pacing block, as a fixture. Written here
// rather than as a constant the arithmetic reads, for the same reason testBands
// is: 1.25, 1.0, 5 and 2 are the user's numbers and they live in routing.yaml.
func testPace() PaceConfig {
	return PaceConfig{
		Enabled:               true,
		ConserveAtRatio:       1.25,
		BlockSpendDownAtRatio: 1.0,
		MinElapsedPct:         5,
		ExpectedOffsetPct:     0, // the offset gets its own tests; 0 keeps the arithmetic legible
		Curve:                 CurveCalendar,
		Location:              time.UTC,
		WeekendWeight:         0.5,
		WeekendPolicy:         WeekendSpendTail,
	}
}

// paceWin builds one wire window with a LENGTH, which is the term pacing needs
// and health does not.
func paceWin(used float64, resetsAt int64, minutes int64) *WireWindow {
	w := &WireWindow{UsedPercent: &Measured{State: MeasuredOk, Value: &used}}
	r := resetsAt
	w.ResetsAt = &r
	if minutes > 0 {
		m := minutes
		w.WindowMinutes = &m
	}
	return w
}

func paceBucket(provider, window string, w *WireWindow, now time.Time) Bucket {
	acct := ""
	return bucketFrom(provider, WireAccount{
		Account: &acct, Label: "default", IsDefault: true, Source: "oauth_poll",
		Windows: WireWindows{FiveHour: w, SevenDay: w, Monthly: w},
	}, window, now)
}

func TestPaceArithmeticAndItsGuards(t *testing.T) {
	now := at(1788126404) // a Tuesday, which the calendar curve does not care about
	fiveHour := int64(300)

	for _, tc := range []struct {
		name    string
		window  *WireWindow
		which   string
		cfg     PaceConfig
		want    PaceState
		known   bool
		ratio   float64
		mention string
		why     string
	}{
		{
			name:   "half a window used at half elapsed is exactly on the curve",
			window: paceWin(50, now.Add(150*time.Minute).Unix(), fiveHour),
			which:  WindowFiveHour, cfg: testPace(),
			want: PaceAhead, known: true, ratio: 1,
			why: "1.00x is AT block_spend_down_at_ratio, and the band is inclusive: a threshold a value can sit exactly on and be treated as below it describes nothing",
		},
		{
			name:   "a quarter used at half elapsed is on track",
			window: paceWin(25, now.Add(150*time.Minute).Unix(), fiveHour),
			which:  WindowFiveHour, cfg: testPace(),
			want: PaceOnTrack, known: true, ratio: 0.5,
		},
		{
			name:   "eighty percent used at half elapsed is overspending",
			window: paceWin(80, now.Add(150*time.Minute).Unix(), fiveHour),
			which:  WindowFiveHour, cfg: testPace(),
			want: PaceOverspending, known: true, ratio: 1.6,
			why: "the case health cannot see at all: 80% used is GREEN against a 90% red band, and the window has half its life left",
		},
		{
			name:   "a window that has ROLLED OVER cannot be paced",
			window: paceWin(80, now.Add(-time.Hour).Unix(), fiveHour),
			which:  WindowFiveHour, cfg: testPace(),
			want: PaceUnknown, mention: "rolled over",
			why: "the founding defect: 67% used against a reset two days in the past would otherwise pace as infinite overspend",
		},
		{
			name:   "a window with NO LENGTH cannot be paced",
			window: paceWin(80, now.Add(time.Hour).Unix(), 0),
			which:  WindowFiveHour, cfg: testPace(),
			want: PaceUnknown, mention: "no window length",
			why: "the monthly overage window, and every provider that reports a percentage and no duration",
		},
		{
			name: "a window with no readable utilization cannot be paced",
			window: func() *WireWindow {
				w := paceWin(0, now.Add(time.Hour).Unix(), fiveHour)
				w.UsedPercent = &Measured{State: MeasuredUnknown, Reason: "the endpoint did not report it"}
				return w
			}(),
			which: WindowFiveHour, cfg: testPace(),
			want: PaceUnknown, mention: "utilization is unreadable",
		},
		{
			name:   "a reset FURTHER OUT than the window is long is refused",
			window: paceWin(10, now.Add(10*time.Hour).Unix(), fiveHour),
			which:  WindowFiveHour, cfg: testPace(),
			want: PaceUnknown, mention: "disagree",
			why: "elapsed would be negative, and a negative denominator flips the comparison the whole feature rests on",
		},
		{
			name:   "the bootstrap FLOOR refuses to judge the first minutes of a window",
			window: paceWin(3, now.Add(295*time.Minute).Unix(), fiveHour),
			which:  WindowFiveHour, cfg: testPace(),
			want: PaceUnknown, mention: "bootstrap floor",
			why: "3% used at 1.7% elapsed is a ratio of 1.8, and without the floor every window would open in CONSERVE",
		},
		{
			name:   "pacing switched off is DISABLED, which is not unknown",
			window: paceWin(80, now.Add(150*time.Minute).Unix(), fiveHour),
			which:  WindowFiveHour,
			cfg: func() PaceConfig {
				c := testPace()
				c.Enabled = false
				return c
			}(),
			want: PaceDisabled,
			why:  "nothing was missing; nobody asked. Collapsing the two would make an operator hunt for a reading that was never wanted",
		},
		{
			name:   "bands that cannot order anything take no verdict",
			window: paceWin(80, now.Add(150*time.Minute).Unix(), fiveHour),
			which:  WindowFiveHour,
			cfg: func() PaceConfig {
				c := testPace()
				c.ConserveAtRatio, c.BlockSpendDownAtRatio = 0, 0
				return c
			}(),
			want: PaceUnknown, mention: "cannot order anything",
		},
		{
			name:   "a block band ABOVE the conserve band is refused rather than applied",
			window: paceWin(80, now.Add(150*time.Minute).Unix(), fiveHour),
			which:  WindowFiveHour,
			cfg: func() PaceConfig {
				c := testPace()
				c.BlockSpendDownAtRatio = 2
				return c
			}(),
			want: PaceUnknown, mention: "cannot order anything",
			why: "a band that conserves without blocking spend-down is two modes at once",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := PaceFor(paceBucket("claude", tc.which, tc.window, now), tc.cfg)
			if got.State != tc.want {
				t.Errorf("state = %q, want %q\nbecause: %s\n%s", got.State, tc.want, got.Because, tc.why)
			}
			if got.Known != tc.known {
				t.Errorf("known = %v, want %v (%s)", got.Known, tc.known, got.Because)
			}
			if tc.known && math.Abs(got.Ratio-tc.ratio) > 0.001 {
				t.Errorf("ratio = %v, want %v (%s)", got.Ratio, tc.ratio, got.Because)
			}
			if tc.mention != "" && !strings.Contains(got.Because, tc.mention) {
				t.Errorf("the explanation does not say why: %q does not mention %q", got.Because, tc.mention)
			}
			if got.Because == "" {
				t.Error("every pace report carries a sentence — a provider that vanishes from an explanation is indistinguishable from one that was considered")
			}
		})
	}
}

// TestTheBootstrapOffsetWidensTheDenominator pins the SECOND bootstrap knob,
// which is not the floor: the floor refuses to answer, the offset answers more
// forgivingly.
func TestTheBootstrapOffsetWidensTheDenominator(t *testing.T) {
	now := at(1788126404)
	// 10% elapsed of a five-hour window (past the 5% floor), 15% used.
	w := paceWin(15, now.Add(270*time.Minute).Unix(), 300)
	b := paceBucket("claude", WindowFiveHour, w, now)

	bare := testPace()
	got := PaceFor(b, bare)
	if !got.Known || math.Abs(got.Ratio-1.5) > 0.001 {
		t.Fatalf("without the offset: ratio = %v (%s)", got.Ratio, got.Because)
	}
	if got.State != PaceOverspending {
		t.Fatalf("without the offset the fleet conserves: state = %q", got.State)
	}

	widened := bare
	widened.ExpectedOffsetPct = 5 // 10% elapsed + 5 points = 15% expected
	got = PaceFor(b, widened)
	if math.Abs(got.Ratio-1.0) > 0.001 {
		t.Fatalf("with a 5-point offset: ratio = %v, want 1.0 (%s)", got.Ratio, got.Because)
	}
	if got.State == PaceOverspending {
		t.Error("the offset is not load-bearing: the same reading still conserves with a five-point wider denominator")
	}
	if !strings.Contains(got.Because, "offset") {
		t.Errorf("the offset moved the answer and the explanation does not say so: %q", got.Because)
	}
}

// TestTheWorkdaysCurveIsTheFiveDayWeek is the whole point of the optional
// curve: a week's allowance spent over five working days is ON PLAN, and the
// calendar curve calls the same reading overspending.
func TestTheWorkdaysCurveIsTheFiveDayWeek(t *testing.T) {
	// A seven-day window that started 00:00 Monday UTC and resets 00:00 the
	// following Monday. `now` is Friday 18:00 — the end of the working week.
	start := time.Date(2026, 8, 31, 0, 0, 0, 0, time.UTC) // Monday
	reset := start.AddDate(0, 0, 7)
	now := time.Date(2026, 9, 4, 18, 0, 0, 0, time.UTC) // Friday
	if now.Weekday() != time.Friday || start.Weekday() != time.Monday {
		t.Fatalf("fixture drift: start=%s now=%s", start.Weekday(), now.Weekday())
	}
	w := paceWin(85, reset.Unix(), 7*24*60)
	b := paceBucket("claude", WindowSevenDay, w, now)

	cal := testPace()
	calendar := PaceFor(b, cal)
	if !calendar.Known {
		t.Fatalf("calendar pace unknown: %s", calendar.Because)
	}
	// 4.75 of 7 days elapsed = 67.9% expected; 85% used = 1.25x.
	if calendar.State != PaceOverspending {
		t.Errorf("the calendar curve should read Friday-evening 85%% as over the line: %s", calendar.Because)
	}

	work := cal
	work.Curve = CurveWorkdays
	work.WeekendWeight = 0.25
	workdays := PaceFor(b, work)
	if !workdays.Known {
		t.Fatalf("workdays pace unknown: %s", workdays.Because)
	}
	if workdays.Curve != CurveWorkdays {
		t.Fatalf("the workdays curve did not answer: curve = %q (%s)", workdays.Curve, workdays.Because)
	}
	if workdays.ExpectedPct <= calendar.ExpectedPct {
		t.Errorf("the workdays curve must expect MORE to be gone by Friday evening than the calendar one (%.1f%% vs %.1f%%) — that is what makes a five-day week on plan",
			workdays.ExpectedPct, calendar.ExpectedPct)
	}
	if workdays.State == PaceOverspending {
		t.Errorf("a week's allowance spent over the working week is on plan under the workdays curve: %s", workdays.Because)
	}

	// And the mirror: the same weight makes SATURDAY spending read as ahead,
	// because the curve says very little was budgeted for it.
	sat := time.Date(2026, 9, 5, 18, 0, 0, 0, time.UTC)
	if sat.Weekday() != time.Saturday {
		t.Fatalf("fixture drift: %s", sat.Weekday())
	}
	satBucket := paceBucket("claude", WindowSevenDay, paceWin(95, reset.Unix(), 7*24*60), sat)
	got := PaceFor(satBucket, work)
	if !got.Known {
		t.Fatalf("saturday pace unknown: %s", got.Because)
	}
	if got.ExpectedPct < workdays.ExpectedPct {
		t.Errorf("the curve must not go BACKWARDS over the weekend: Friday %.1f%%, Saturday %.1f%%", workdays.ExpectedPct, got.ExpectedPct)
	}
}

// TestAZeroWeekendWeightCannotDivideByZero is the guard the "safe nonzero
// weekend weight" rule exists for, exercised at the arithmetic rather than only
// at load time.
func TestAZeroWeekendWeightCannotDivideByZero(t *testing.T) {
	// A seven-day window is always mostly weekdays, so make the DANGEROUS case
	// directly: weightedSeconds over a Saturday with a zero weight.
	sat := time.Date(2026, 9, 5, 0, 0, 0, 0, time.UTC)
	cfg := testPace()
	cfg.Curve, cfg.WeekendWeight = CurveWorkdays, 0
	if got := weightedSeconds(sat, sat.Add(12*time.Hour), cfg); got != 0 {
		t.Fatalf("a zero weekend weight integrates to %v, not 0 — the fixture is not exercising the hole", got)
	}

	start := time.Date(2026, 8, 31, 0, 0, 0, 0, time.UTC)
	now := time.Date(2026, 9, 4, 18, 0, 0, 0, time.UTC)
	b := paceBucket("claude", WindowSevenDay, paceWin(85, start.AddDate(0, 0, 7).Unix(), 7*24*60), now)

	got := PaceFor(b, cfg)
	if got.Curve != CurveCalendar {
		t.Errorf("an unusable weekend weight must fall back to the calendar curve, got %q", got.Curve)
	}
	if !strings.Contains(got.Because, "weekend_weight") {
		t.Errorf("the fallback must SAY it happened, or a matrix value that silently does nothing looks like one that works: %q", got.Because)
	}
	if math.IsNaN(got.Ratio) || math.IsInf(got.Ratio, 0) {
		t.Errorf("ratio = %v — the zero-weight hole produced a non-number", got.Ratio)
	}
}

// TestTheWeekendReserveTightensTheCurveAndSpendTailDoesNot proves both settings
// of the one knob whose two values are not two numbers.
func TestTheWeekendReserveTightensTheCurveAndSpendTailDoesNot(t *testing.T) {
	start := time.Date(2026, 8, 31, 0, 0, 0, 0, time.UTC)
	now := time.Date(2026, 9, 3, 12, 0, 0, 0, time.UTC) // Thursday
	b := paceBucket("claude", WindowSevenDay, paceWin(60, start.AddDate(0, 0, 7).Unix(), 7*24*60), now)

	tail := testPace()
	tail.WeekendPolicy, tail.WeekendReservePct = WeekendSpendTail, 20
	spendTail := PaceFor(b, tail)
	if !strings.Contains(spendTail.Because, "IGNORED") {
		t.Errorf("a reserve under spend_tail changes nothing and must SAY so: %q", spendTail.Because)
	}

	reserve := tail
	reserve.WeekendPolicy = WeekendReserve
	held := PaceFor(b, reserve)
	if held.ExpectedPct >= spendTail.ExpectedPct {
		t.Errorf("reserving 20%% must tighten the curve: reserve %.1f%% vs spend_tail %.1f%%", held.ExpectedPct, spendTail.ExpectedPct)
	}
	if held.Ratio <= spendTail.Ratio {
		t.Errorf("a tighter curve must read the SAME consumption as further over it: %.2fx vs %.2fx", held.Ratio, spendTail.Ratio)
	}
	if !strings.Contains(held.Because, "reserve") {
		t.Errorf("the reserve moved the answer and the explanation does not say so: %q", held.Because)
	}

	// A reserve of 0 under `reserve` is a no-op, and says so rather than
	// scaling by 1.0 silently.
	none := reserve
	none.WeekendReservePct = 0
	if got := PaceFor(b, none); math.Abs(got.ExpectedPct-spendTail.ExpectedPct) > 0.001 {
		t.Errorf("a zero reserve must leave the curve alone: %.3f vs %.3f", got.ExpectedPct, spendTail.ExpectedPct)
	}
}

// TestWorstPaceIsTheWorseOfTheReadableWindows is §4's rule: an Anthropic
// decision takes the worse of the five-hour and the seven-day pace, and a
// provider whose five-hour window is stale is answered by the one that is not.
func TestWorstPaceIsTheWorseOfTheReadableWindows(t *testing.T) {
	now := at(1788126404)
	acct := ""
	// Five-hour: 20% used at 50% elapsed = 0.4x. Seven-day: 60% used at 50%
	// elapsed = 1.2x. The weekly window binds.
	claude := WireAccount{
		Account: &acct, Label: "default", IsDefault: true, Source: "oauth_poll",
		Windows: WireWindows{
			FiveHour: paceWin(20, now.Add(150*time.Minute).Unix(), 300),
			SevenDay: paceWin(60, now.Add(84*time.Hour).Unix(), 7*24*60),
			Monthly:  nil,
		},
	}
	var buckets []Bucket
	for _, w := range WindowOrder {
		buckets = append(buckets, bucketFrom("claude", claude, w, now))
	}

	worst, all := PaceForAccount(buckets, "claude", "", testPace())
	if len(all) != 3 {
		t.Fatalf("every window is reported, got %d", len(all))
	}
	if worst.Window != WindowSevenDay {
		t.Errorf("the WORSE window must bind: got %s at %.2fx", worst.Window, worst.Ratio)
	}
	if !worst.BlocksSpendDown() {
		t.Errorf("1.20x is above the block band and must block spend-down: %s", worst.Because)
	}

	// Now make the five-hour window the bad one and check it wins instead — the
	// fold must have no favourite window.
	claude.Windows.FiveHour = paceWin(95, now.Add(150*time.Minute).Unix(), 300)
	buckets = nil
	for _, w := range WindowOrder {
		buckets = append(buckets, bucketFrom("claude", claude, w, now))
	}
	worst, _ = PaceForAccount(buckets, "claude", "", testPace())
	if worst.Window != WindowFiveHour {
		t.Errorf("the five-hour window is now the worse one and must bind: got %s at %.2fx", worst.Window, worst.Ratio)
	}
	if !worst.Conserves() {
		t.Errorf("1.90x is above the conserve band: %s", worst.Because)
	}
}

// TestAProviderWithNoPaceableWindowStaysConservative is the copilot case: it
// publishes nothing, so pace conserves nothing and blocks nothing, and the
// answer says which.
func TestAProviderWithNoPaceableWindowStaysConservative(t *testing.T) {
	now := at(1788126404)
	acct := ""
	dark := WireAccount{
		Account: &acct, Label: "copilot", IsDefault: true, Source: "disk",
		Windows: WireWindows{},
	}
	var buckets []Bucket
	for _, w := range WindowOrder {
		buckets = append(buckets, bucketFrom("copilot", dark, w, now))
	}
	worst, all := PaceForAccount(buckets, "copilot", "", testPace())
	if worst.Known || worst.Conserves() || worst.BlocksSpendDown() {
		t.Fatalf("an unreadable provider must neither conserve nor block: %+v", worst)
	}
	if worst.Because == "" || len(all) != 3 {
		t.Fatalf("it must still be explained, per window: %+v", all)
	}
}

// TestAStaleReadingCannotBePaced is the second SHOULD-FIX: a reading whose
// window is CURRENT (resets_at is still in the future — the currency guard
// alone would wave it through) but whose daemon-reported freshness is false
// must not produce a known pace. Live, a fresh:false reading with observed_at
// 72 hours old on a still-current seven-day window paced as known/on_track at
// 0.228x (a stale 20% used divided by a live 86% elapsed) — under-conserving
// exactly the way a stale currency reading would, just through the other
// input to the division.
func TestAStaleReadingCannotBePaced(t *testing.T) {
	now := at(1788126404)
	acct := ""
	fresh := false
	observed := now.Add(-72 * time.Hour).Unix()
	staleAccount := WireAccount{
		Account: &acct, Label: "default", IsDefault: true, Source: "disk",
		Fresh: &fresh, ObservedAt: &observed,
		Windows: WireWindows{SevenDay: paceWin(20, now.Add(86*time.Hour).Unix(), 7*24*60)},
	}
	b := bucketFrom("claude", staleAccount, WindowSevenDay, now)

	got := PaceFor(b, testPace())
	if got.Known {
		t.Fatalf("a stale reading produced a KNOWN pace: %+v", got)
	}
	if got.State != PaceUnknown {
		t.Errorf("state = %q, want unknown", got.State)
	}
	if got.Conserves() || got.BlocksSpendDown() {
		t.Errorf("a stale reading must neither add conserve nor block spend-down: %+v", got)
	}
	if !strings.Contains(got.Because, "stale") {
		t.Errorf("the explanation does not name staleness: %q", got.Because)
	}
	if !strings.Contains(got.Because, "72h") {
		t.Errorf("the explanation should say how old the evidence is: %q", got.Because)
	}

	// The worse-of-two fold must skip a stale account exactly as it skips a
	// provider with no paceable window at all: both windows share the same
	// account-level Fresh flag, so the whole account folds to unknown.
	staleAccount.Windows.FiveHour = paceWin(90, now.Add(2*time.Hour).Unix(), 300)
	var buckets []Bucket
	for _, w := range WindowOrder {
		buckets = append(buckets, bucketFrom("claude", staleAccount, w, now))
	}
	worst, all := PaceForAccount(buckets, "claude", "", testPace())
	if worst.Known || worst.Conserves() || worst.BlocksSpendDown() {
		t.Fatalf("a stale account must fold to an unknown pace that neither conserves nor blocks: %+v", worst)
	}
	if len(all) != 3 {
		t.Fatalf("every window is still reported: %+v", all)
	}
}

// TestWeightedSecondsCrossesADSTTransitionAtLocalMidnight is the SHOULD-FIX
// case a deep review caught: America/Santiago moves its clocks at local
// midnight (spring-forward, first Saturday of September), so the day walk's
// `time.Date(y, m, d+1, 0,0,0,0, loc)` asks for a wall-clock instant that does
// not exist. Live, requesting 2026-09-06 00:00 in that zone normalizes
// BACKWARDS to 2026-09-05 23:00:00 — not a day past `cur` — which used to trip
// `!next.After(cur)` and make the whole call return 0, silently degrading the
// workdays curve to a phantom "no expected progress at all" for any window
// spanning the transition. Cuba, Paraguay, Lebanon and Azerbaijan carry the
// same shape of transition and would trip the same stall.
//
// The proof is a brute-force per-minute integration over the same interval:
// if the day walk is choosing the right weight for the right seconds, the two
// must agree to within a minute of rounding.
func TestWeightedSecondsCrossesADSTTransitionAtLocalMidnight(t *testing.T) {
	loc, err := time.LoadLocation("America/Santiago")
	if err != nil {
		t.Skipf("tzdata for America/Santiago is not available in this environment: %v", err)
	}
	from := time.Date(2026, 9, 5, 0, 0, 0, 0, loc) // Saturday, the transition date
	to := time.Date(2026, 9, 7, 0, 0, 0, 0, loc)   // Monday
	if from.Weekday() != time.Saturday {
		t.Fatalf("fixture drift: America/Santiago no longer moves its clocks on 2026-09-06 (from.Weekday = %s)", from.Weekday())
	}

	cfg := testPace()
	cfg.Curve, cfg.Location, cfg.WeekendWeight = CurveWorkdays, loc, 0.5

	got := weightedSeconds(from, to, cfg)
	if got <= 0 {
		t.Fatalf("weightedSeconds across the Santiago DST transition = %v, want a positive total — the stall this test exists to catch returns exactly 0", got)
	}

	want := bruteForceWeightedSeconds(from, to, loc, cfg.WeekendWeight)
	if math.Abs(got-want) > 60 {
		t.Errorf("weightedSeconds = %v, brute-force per-minute integration = %v — they disagree by more than 60 seconds", got, want)
	}
}

// bruteForceWeightedSeconds is the independent check for the DST test above:
// it walks the same interval one minute at a time, off time.Time.Weekday
// directly, and cannot share the day-walk bug because it never computes a day
// boundary at all.
func bruteForceWeightedSeconds(from, to time.Time, loc *time.Location, weekendWeight float64) float64 {
	total := 0.0
	step := time.Minute
	for t := from; t.Before(to); t = t.Add(step) {
		w := 1.0
		switch t.In(loc).Weekday() {
		case time.Saturday, time.Sunday:
			w = weekendWeight
		}
		remaining := to.Sub(t)
		s := step
		if remaining < s {
			s = remaining
		}
		total += w * s.Seconds()
	}
	return total
}

// TestPaceNeverReadsTheUngatedScalar is window.go's rule, restated for the file
// that does the dividing: a stale reading must not be able to reach the
// numerator through a second door.
func TestPaceNeverReadsTheUngatedScalar(t *testing.T) {
	raw, err := readSource("pace.go")
	if err != nil {
		t.Fatalf("read pace.go: %v", err)
	}
	// The CALL form, not the name: the file's own header explains at length why
	// it must not read that scalar, and a guard that could not tell an
	// explanation from a call would force the explanation out of the file.
	if strings.Contains(raw, ".DisplayOnlyRawUsedPercent(") || strings.Contains(raw, ".rawUsedPercent") {
		t.Error("pace.go reaches the UNGATED raw usage scalar. A pace ratio taken off a window that closed two days ago is the founding defect with a division added to it — read through Bucket.Reading, which refuses one.")
	}
}
