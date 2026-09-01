package routing

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// ---------------------------------------------------------------------------
// THE WIRING: every knob in the file reaches the arithmetic
//
// This fleet's most common bug is a setting that is WRITTEN and never READ, and
// it looks exactly like a working one. The two tests below make that shape fail:
// the first mutates each knob and requires the mapped PaceConfig to move with
// it, and the second requires the first's table to cover every field the struct
// has, so a knob added later cannot be silently left unmapped.
// ---------------------------------------------------------------------------

func matrixWithPacing(t *testing.T, mutate func(*Pacing)) *Matrix {
	t.Helper()
	m := shipped(t)
	p := m.Thresholds.Pacing
	if mutate != nil {
		mutate(&p)
	}
	m.Thresholds.Pacing = p
	return m
}

type pacingKnob struct {
	// fields are the struct field names this row is responsible for.
	fields []string
	mutate func(*Pacing)
	// moved reports whether the mapped config reflects the mutation.
	moved func(limits.PaceConfig) bool
}

func pacingKnobs() []pacingKnob {
	off := false
	return []pacingKnob{
		{[]string{"Enabled"}, func(p *Pacing) { p.Enabled = &off }, func(c limits.PaceConfig) bool { return !c.Enabled }},
		{[]string{"ConserveAtRatio"}, func(p *Pacing) { p.ConserveAtRatio = 3.5 }, func(c limits.PaceConfig) bool { return c.ConserveAtRatio == 3.5 }},
		{[]string{"BlockSpendDownAtRatio"}, func(p *Pacing) { p.BlockSpendDownAtRatio = 0.75 }, func(c limits.PaceConfig) bool { return c.BlockSpendDownAtRatio == 0.75 }},
		{[]string{"Bootstrap", "MinElapsedPct"}, func(p *Pacing) { p.Bootstrap.MinElapsedPct = 33 }, func(c limits.PaceConfig) bool { return c.MinElapsedPct == 33 }},
		{[]string{"ExpectedOffsetPct"}, func(p *Pacing) { p.Bootstrap.ExpectedOffsetPct = 7 }, func(c limits.PaceConfig) bool { return c.ExpectedOffsetPct == 7 }},
		{[]string{"SevenDay", "Curve"}, func(p *Pacing) { p.SevenDay.Curve = "WorkDays" }, func(c limits.PaceConfig) bool { return c.Curve == limits.CurveWorkdays }},
		{[]string{"Timezone"}, func(p *Pacing) { p.SevenDay.Timezone = "Asia/Tokyo" }, func(c limits.PaceConfig) bool {
			return c.Location != nil && c.Location.String() == "Asia/Tokyo"
		}},
		{[]string{"WeekendWeight"}, func(p *Pacing) { p.SevenDay.WeekendWeight = 0.125 }, func(c limits.PaceConfig) bool { return c.WeekendWeight == 0.125 }},
		{[]string{"Weekend"}, func(p *Pacing) { p.SevenDay.Weekend = "Reserve" }, func(c limits.PaceConfig) bool { return c.WeekendPolicy == limits.WeekendReserve }},
		{[]string{"WeekendReservePct"}, func(p *Pacing) { p.SevenDay.WeekendReservePct = 17 }, func(c limits.PaceConfig) bool { return c.WeekendReservePct == 17 }},
	}
}

func TestEveryPacingKnobReachesTheArithmetic(t *testing.T) {
	for _, knob := range pacingKnobs() {
		name := strings.Join(knob.fields, ".")
		t.Run(name, func(t *testing.T) {
			m := matrixWithPacing(t, knob.mutate)
			if !knob.moved(m.PaceConfig()) {
				t.Errorf("routing.yaml's thresholds.pacing.%s is parsed and never reaches limits.PaceConfig — a setting that is written and never read looks exactly like a working one: %+v", name, m.PaceConfig())
			}
		})
	}
}

// TestThePacingKnobTableIsComplete is the guard on the guard: a field added to
// the config struct without a row above would otherwise be untested AND
// unmapped, which is the failure the table exists to catch.
func TestThePacingKnobTableIsComplete(t *testing.T) {
	covered := map[string]bool{}
	for _, k := range pacingKnobs() {
		for _, f := range k.fields {
			covered[f] = true
		}
	}
	var walk func(reflect.Type)
	seen := 0
	walk = func(ty reflect.Type) {
		for i := 0; i < ty.NumField(); i++ {
			f := ty.Field(i)
			seen++
			if !covered[f.Name] {
				t.Errorf("thresholds.pacing carries field %q that no pacingKnobs() row mutates — add a row, and check the value actually arrives in limits.PaceConfig", f.Name)
			}
			if f.Type.Kind() == reflect.Struct {
				walk(f.Type)
			}
		}
	}
	walk(reflect.TypeOf(Pacing{}))
	if seen < 10 {
		t.Fatalf("walked only %d pacing fields — the struct was renamed and this guard is guarding nothing", seen)
	}
}

// TestTheShippedPacingDefaultsAreTheDocumentedOnes pins what a fresh install
// gets. The numbers are the file's, and this is the test that fails when the
// shipped yaml and the documentation drift apart.
func TestTheShippedPacingDefaultsAreTheDocumentedOnes(t *testing.T) {
	m := shipped(t)
	p := m.Thresholds.Pacing
	if !p.IsEnabled() {
		t.Error("pacing ships ON; the OFF switch is for a fleet whose rhythm does not fit a curve")
	}
	if p.ConserveAtRatio != 1.25 || p.BlockSpendDownAtRatio != 1.0 {
		t.Errorf("shipped bands moved: conserve %v, block %v", p.ConserveAtRatio, p.BlockSpendDownAtRatio)
	}
	if p.Bootstrap.MinElapsedPct != 5 || p.Bootstrap.ExpectedOffsetPct != 2 {
		t.Errorf("shipped bootstrap moved: %+v", p.Bootstrap)
	}
	if p.SevenDay.Curve != limits.CurveCalendar {
		t.Errorf("the CALENDAR curve ships: a fleet that works at the weekend must not be told to conserve on Saturday by default, got %q", p.SevenDay.Curve)
	}
	if p.SevenDay.Timezone != TimezoneLocal {
		t.Errorf("the shipped timezone is the host's own, got %q", p.SevenDay.Timezone)
	}
	if !(p.SevenDay.WeekendWeight > 0) {
		t.Errorf("the shipped weekend weight must be a SAFE nonzero value — at zero a window ending over a weekend has no expected progress at all, got %v", p.SevenDay.WeekendWeight)
	}
	if p.SevenDay.Weekend != limits.WeekendSpendTail || p.SevenDay.WeekendReservePct != 0 {
		t.Errorf("the shipped weekend policy is spend_tail with reserve 0, got %q / %v", p.SevenDay.Weekend, p.SevenDay.WeekendReservePct)
	}
	// And the shipped file must not report itself as broken.
	for _, issue := range m.Issues {
		if strings.HasPrefix(issue.Where, "thresholds.pacing") {
			t.Errorf("the shipped matrix reports its own pacing block as an issue: %s", issue)
		}
	}
}

func TestPacingValidationReportsUnusableBlocks(t *testing.T) {
	for _, tc := range []struct {
		name   string
		mutate func(*Pacing)
		want   string
	}{
		{"a conserve band of zero orders nothing", func(p *Pacing) { p.ConserveAtRatio = 0 }, "positive ratio"},
		{"a conserve band below 1.0 conserves permanently", func(p *Pacing) { p.ConserveAtRatio = 0.8; p.BlockSpendDownAtRatio = 0.5 }, "BELOW 1.0"},
		{"a block band above the conserve band is two modes at once", func(p *Pacing) { p.BlockSpendDownAtRatio = 2 }, "ABOVE conserve_at_ratio"},
		{"no bootstrap floor opens every window in conserve", func(p *Pacing) { p.Bootstrap.MinElapsedPct = 0 }, "first second"},
		{"a floor no window reaches is never judged", func(p *Pacing) { p.Bootstrap.MinElapsedPct = 150 }, "no window ever reaches"},
		{"a negative offset narrows the denominator", func(p *Pacing) { p.Bootstrap.ExpectedOffsetPct = -5 }, "NARROWS"},
		{"an unknown curve", func(p *Pacing) { p.SevenDay.Curve = "sinusoidal" }, "not a seven-day curve"},
		{"a timezone this host cannot resolve", func(p *Pacing) { p.SevenDay.Timezone = "Mars/Olympus" }, "cannot be resolved"},
		{"a zero weekend weight under the workdays curve", func(p *Pacing) { p.SevenDay.Curve = limits.CurveWorkdays; p.SevenDay.WeekendWeight = 0 }, "infinite overspend"},
		{"a reserve that is ignored under spend_tail", func(p *Pacing) { p.SevenDay.WeekendReservePct = 30 }, "IGNORED"},
		{"a reserve policy that reserves nothing", func(p *Pacing) { p.SevenDay.Weekend = limits.WeekendReserve; p.SevenDay.WeekendReservePct = 0 }, "nothing is reserved"},
		{"an unknown weekend policy", func(p *Pacing) { p.SevenDay.Weekend = "hibernate" }, "not a weekend policy"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			p := shipped(t).Thresholds.Pacing
			tc.mutate(&p)
			issues := validatePacing(p)
			found := false
			for _, i := range issues {
				if strings.Contains(i.Detail, tc.want) {
					found = true
				}
				if !strings.HasPrefix(i.Where, "thresholds.pacing") {
					t.Errorf("an issue must point at the block it is about, got %q", i.Where)
				}
			}
			if !found {
				t.Errorf("no issue mentioning %q; got %v", tc.want, issues)
			}
		})
	}
}

// TestPacingOffIsNotValidated is the other half: `enabled: false` is a
// legitimate configuration and must not fill an operator's log with complaints
// about bands nothing reads.
func TestPacingOffIsNotValidated(t *testing.T) {
	off := false
	p := Pacing{Enabled: &off} // every band zero, every word empty
	if issues := validatePacing(p); len(issues) != 0 {
		t.Errorf("pacing that is switched off reports %d issue(s) about rules it does not apply: %v", len(issues), issues)
	}
}

// ---------------------------------------------------------------------------
// THE DECISION
// ---------------------------------------------------------------------------

// paceWindows is the shape that makes pacing bite: real window LENGTHS, which
// is the term claudemon now reports for Anthropic as well as for Codex.
func paceWindows(fiveUsed float64, fiveResets time.Duration, sevenUsed float64, sevenResets time.Duration) map[string]winSpec {
	return map[string]winSpec{
		limits.WindowFiveHour: {used: fiveUsed, resets: fiveResets, minutes: 300},
		limits.WindowSevenDay: {used: sevenUsed, resets: sevenResets, minutes: 7 * 24 * 60},
	}
}

func selectWith(t *testing.T, m *Matrix, snap limits.Snapshot, req Request) Decision {
	t.Helper()
	return Select(m, snap, nil, policyNow, req)
}

func modeOf(t *testing.T, m *Matrix, snap limits.Snapshot, provider string) Decision {
	t.Helper()
	return selectWith(t, m, snap, Request{Role: "implementer", Provider: provider, ForecastDemandBeforeResetPct: floatPtr(0)})
}

func floatPtr(v float64) *float64 { return &v }

// TestPaceAddsConserveWhereHealthCannotSeeIt is the feature in one case: 80% of
// a five-hour window gone with half the window left is GREEN-to-YELLOW on the
// health ladder and is a fleet that will be out of allowance in an hour.
func TestPaceAddsConserveWhereHealthCannotSeeIt(t *testing.T) {
	m := shipped(t)
	// 60% of the five-hour window used at 50% elapsed = 1.20x… under the 1.25
	// band. 80% used is 1.6x and over it. Both are the same HEALTH (yellow).
	under := modeOf(t, m, snapshotOf(t, "claude", "", paceWindows(60, 150*time.Minute, 10, 84*time.Hour)), "claude")
	over := modeOf(t, m, snapshotOf(t, "claude", "", paceWindows(80, 150*time.Minute, 10, 84*time.Hour)), "claude")

	if under.Mode == ModeConserve {
		t.Errorf("1.20x is under the shipped 1.25 band and must not conserve: %v", under.Reason)
	}
	if over.Mode != ModeConserve {
		t.Fatalf("1.60x must conserve; got %s\n%v", over.Mode, over.Reason)
	}
	if over.Capacity.Health != limits.HealthYellow {
		t.Fatalf("fixture drift: the point is that HEALTH is the same on both sides, got %s", over.Capacity.Health)
	}
	if over.Capacity.Pace == nil || !over.Capacity.Pace.Conserves() {
		t.Fatalf("the decision must carry the pace it acted on: %+v", over.Capacity.Pace)
	}
	if len(over.Capacity.PaceWindows) == 0 {
		t.Error("every window's pace is reported, so a reader can see which one bound")
	}
	joined := strings.Join(over.Reason, " | ")
	if !strings.Contains(joined, "pace") || !strings.Contains(joined, "faster than it refills") {
		t.Errorf("the reason list must EXPLAIN the conserve, not just assert it: %v", over.Reason)
	}
}

// TestAnthropicTakesTheWorseOfItsTwoWindows: both Anthropic windows now carry a
// length, so both are paced and the worse one binds — whichever it is.
func TestAnthropicTakesTheWorseOfItsTwoWindows(t *testing.T) {
	m := shipped(t)

	// The WEEKLY window is the bad one: 70% used halfway through the week
	// (1.40x) while the five-hour window is comfortable.
	weekly := modeOf(t, m, snapshotOf(t, "claude", "", paceWindows(10, 150*time.Minute, 70, 84*time.Hour)), "claude")
	if weekly.Mode != ModeConserve {
		t.Fatalf("the weekly window's pace must bind: %s\n%v", weekly.Mode, weekly.Reason)
	}
	if weekly.Capacity.Pace.Window != limits.WindowSevenDay {
		t.Errorf("the binding window must be NAMED: got %q", weekly.Capacity.Pace.Window)
	}

	// The FIVE-HOUR window is the bad one, and the weekly one is fine.
	fast := modeOf(t, m, snapshotOf(t, "claude", "", paceWindows(80, 150*time.Minute, 10, 84*time.Hour)), "claude")
	if fast.Capacity.Pace.Window != limits.WindowFiveHour {
		t.Errorf("the five-hour window is the worse one here and must bind: got %q", fast.Capacity.Pace.Window)
	}

	// A five-hour window that has ROLLED OVER cannot bind, whatever it said —
	// which is how Codex, whose five-hour reading is frequently stale, ends up
	// answered by its seven-day pace without a special case for it.
	stale := snapshotOf(t, "codex", "", map[string]winSpec{
		limits.WindowFiveHour: {used: 95, resets: -48 * time.Hour, minutes: 300},
		limits.WindowSevenDay: {used: 70, resets: 84 * time.Hour, minutes: 7 * 24 * 60},
	})
	d := modeOf(t, m, stale, "codex")
	if d.Capacity.Pace.Window != limits.WindowSevenDay {
		t.Errorf("a rolled-over five-hour window must not be paceable: bound %q at %.2fx", d.Capacity.Pace.Window, d.Capacity.Pace.Ratio)
	}
}

// TestPaceNeverOverridesRedOrExhaustedHealth is the rule that keeps pacing
// additive. A flattering ratio must not be able to talk a scarce allowance back
// to normal.
func TestPaceNeverOverridesRedOrExhaustedHealth(t *testing.T) {
	m := shipped(t)
	for _, tc := range []struct {
		name string
		used float64
	}{
		{"red", 95},
		{"exhausted", 100},
	} {
		t.Run(tc.name, func(t *testing.T) {
			// Used almost exactly in step with the clock: pace is ON TRACK, and
			// the provider is still nearly out of allowance.
			snap := snapshotOf(t, "claude", "", map[string]winSpec{
				limits.WindowFiveHour: {used: tc.used, resets: 3 * time.Minute, minutes: 300},
			})
			d := modeOf(t, m, snap, "claude")
			if d.Mode != ModeConserve {
				t.Fatalf("a %s allowance conserves whatever the pace says: got %s\n%v", tc.name, d.Mode, d.Reason)
			}
			joined := strings.Join(d.Reason, " | ")
			if !strings.Contains(joined, "allowance is "+tc.name) {
				t.Errorf("the HEALTH reason must be the one given, not a pace one: %v", d.Reason)
			}
			if strings.Contains(joined, "window pace:") {
				t.Errorf("the pace arm must not even be reached on a %s provider — it sits after the health arms so it can only ever ADD conserve: %v", tc.name, d.Reason)
			}
		})
	}
}

// paceBlockedSpendDown is the realistic shape of the block: a five-hour window
// resetting soon with most of it left, against a weekly window being consumed
// exactly on schedule. Before pacing, that is a textbook spend-down.
func paceBlockedSpendDown() map[string]winSpec {
	// 60% of the week gone at the halfway mark is 1.15x against the shipped
	// curve — over the block band, under the conserve one — while the
	// five-hour window resets in an hour with 80% of it left.
	return paceWindows(20, 60*time.Minute, 60, 84*time.Hour)
}

func TestPaceBlocksSpendDownWithoutConserving(t *testing.T) {
	m := shipped(t)
	d := modeOf(t, m, snapshotOf(t, "claude", "", paceBlockedSpendDown()), "claude")

	if d.Mode == ModeSpendDown {
		t.Fatalf("what is left is already spoken for and must not be spent early: %v", d.Reason)
	}
	if d.Mode != ModeNormal {
		t.Fatalf("blocking a spend-down is not conserving: got %s\n%v", d.Mode, d.Reason)
	}
	if d.Capacity.Pace == nil || !d.Capacity.Pace.BlocksSpendDown() || d.Capacity.Pace.Conserves() {
		t.Fatalf("the middle band is the claim: %+v", d.Capacity.Pace)
	}
	if !strings.Contains(strings.Join(d.Reason, " | "), "not going to expire unused") {
		t.Errorf("the block must be explained: %v", d.Reason)
	}
}

// TestPacingDisabledReproducesThePrePacingAnswer is requirement 7, and it is
// the reason the whole feature is switchable: with `enabled: false` the answer
// is the one this router gave before pacing existed, down to the absence of the
// fields.
func TestPacingDisabledReproducesThePrePacingAnswer(t *testing.T) {
	off := false
	for _, tc := range []struct {
		name    string
		windows map[string]winSpec
		// wantOn is the mode WITH pacing; wantOff is the pre-pacing mode.
		wantOn, wantOff Mode
	}{
		{"the pace conserve arm", paceWindows(80, 150*time.Minute, 10, 84*time.Hour), ModeConserve, ModeNormal},
		{"the pace spend-down block", paceBlockedSpendDown(), ModeNormal, ModeSpendDown},
	} {
		t.Run(tc.name, func(t *testing.T) {
			snap := snapshotOf(t, "claude", "", tc.windows)
			on := modeOf(t, shipped(t), snap, "claude")
			if on.Mode != tc.wantOn {
				t.Fatalf("with pacing: mode = %s, want %s\n%v", on.Mode, tc.wantOn, on.Reason)
			}

			m := matrixWithPacing(t, func(p *Pacing) { p.Enabled = &off })
			d := modeOf(t, m, snap, "claude")
			if d.Mode != tc.wantOff {
				t.Fatalf("with pacing OFF: mode = %s, want the pre-pacing %s\n%v", d.Mode, tc.wantOff, d.Reason)
			}
			if d.Capacity.Pace != nil || d.Capacity.PaceWindows != nil {
				t.Errorf("a decision made with pacing off must carry no pace fields at all: %+v", d.Capacity.Pace)
			}
			for _, r := range d.Reason {
				if strings.Contains(r, "pace") {
					t.Errorf("a decision made with pacing off must not mention pacing: %q", r)
				}
			}
			// And the answer must serialize without the pacing keys, because a
			// client reading `pace` off an answer that did not use one would be
			// reading a number nobody computed.
			raw, err := json.Marshal(d)
			if err != nil {
				t.Fatalf("marshal: %v", err)
			}
			for _, key := range []string{`"pace"`, `"paceWindows"`} {
				if strings.Contains(string(raw), key) {
					t.Errorf("the serialized decision still carries %s with pacing off", key)
				}
			}
		})
	}
}

// ---------------------------------------------------------------------------
// MUTATION TESTS
//
// Go cannot recompile a mutated policy inside its own test binary, so these
// mutate the GUARD'S INPUT instead: for each guard, the mutation that would
// disable it is applied and the verdict is required to move. A guard whose
// removal changes nothing is not protecting anything, and that is the failure
// mode these catch — a threshold that is read, compared and never able to bite.
// ---------------------------------------------------------------------------

func TestPaceGuardsAreLoadBearing(t *testing.T) {
	// The baseline: a decision that conserves BECAUSE of pace and for no other
	// reason. Every mutation below is expected to take the conserve away.
	base := snapshotOf(t, "claude", "", paceWindows(80, 150*time.Minute, 10, 84*time.Hour))
	if got := modeOf(t, shipped(t), base, "claude").Mode; got != ModeConserve {
		t.Fatalf("the baseline must conserve on pace alone, got %s", got)
	}

	for _, tc := range []struct {
		guard  string
		mutate func(*Pacing)
		// snapshot mutations, for the guards that live in the READING rather
		// than in the configuration.
		windows map[string]winSpec
		why     string
	}{
		{
			guard:  "the conserve band",
			mutate: func(p *Pacing) { p.ConserveAtRatio = 5 },
			why:    "conserve_at_ratio is compared and cannot bite — the band is decoration",
		},
		{
			guard:  "the bootstrap floor",
			mutate: func(p *Pacing) { p.Bootstrap.MinElapsedPct = 80 },
			why:    "min_elapsed_pct never suppresses a verdict, so a window's first minutes can still read as a crisis",
		},
		{
			guard:  "the bootstrap offset",
			mutate: func(p *Pacing) { p.Bootstrap.ExpectedOffsetPct = 20 },
			why:    "expected_offset_pct never widens the denominator, so it is a number in a file that changes nothing",
		},
		{
			guard:  "the enabled switch",
			mutate: func(p *Pacing) { off := false; p.Enabled = &off },
			why:    "enabled: false does not switch pacing off, so there is no way back to the pre-pacing answers",
		},
		{
			guard:   "the currency guard",
			windows: map[string]winSpec{limits.WindowFiveHour: {used: 80, resets: -time.Hour, minutes: 300}},
			why:     "a window that CLOSED an hour ago can still produce a pace ratio — the founding defect with a division added to it",
		},
		{
			guard:   "the window-length requirement",
			windows: map[string]winSpec{limits.WindowFiveHour: {used: 80, resets: 150 * time.Minute}},
			why:     "a window with no declared length is being paced against a denominator nobody reported",
		},
	} {
		t.Run(tc.guard, func(t *testing.T) {
			m := shipped(t)
			if tc.mutate != nil {
				m = matrixWithPacing(t, tc.mutate)
			}
			snap := base
			if tc.windows != nil {
				snap = snapshotOf(t, "claude", "", tc.windows)
			}
			d := modeOf(t, m, snap, "claude")
			if d.Mode == ModeConserve {
				t.Errorf("removing %s left the answer unchanged (still CONSERVE), so %s\n%v", tc.guard, tc.why, d.Reason)
			}
		})
	}
}

// TestTheSpendDownBlockGuardIsLoadBearing is the same treatment for the middle
// band, which has its own failure mode: a block band that never fires makes
// spend-down unconditional again.
func TestTheSpendDownBlockGuardIsLoadBearing(t *testing.T) {
	snap := snapshotOf(t, "claude", "", paceBlockedSpendDown())
	if got := modeOf(t, shipped(t), snap, "claude").Mode; got != ModeNormal {
		t.Fatalf("the baseline must be a BLOCKED spend-down, got %s", got)
	}
	m := matrixWithPacing(t, func(p *Pacing) { p.BlockSpendDownAtRatio = 1.25 })
	if got := modeOf(t, m, snap, "claude").Mode; got != ModeSpendDown {
		t.Errorf("raising block_spend_down_at_ratio above the reading must restore the spend-down, got %s — the band is not what blocked it", got)
	}
}

// TestTheWorkdayCurveIsLoadBearingEndToEnd walks the curve knob all the way
// from routing.yaml to a decision, because that is the knob most likely to be
// parsed, validated, reported and never actually applied.
func TestTheWorkdayCurveIsLoadBearingEndToEnd(t *testing.T) {
	// A Friday-evening decision, mid-week-window: the calendar curve says this
	// is over the line and the five-workday curve says it is on plan.
	friday := time.Date(2026, 9, 4, 18, 0, 0, 0, time.UTC)
	if friday.Weekday() != time.Friday {
		t.Fatalf("fixture drift: %s", friday.Weekday())
	}
	reset := time.Date(2026, 9, 7, 0, 0, 0, 0, time.UTC) // the following Monday
	snap := func(t *testing.T) limits.Snapshot {
		t.Helper()
		doc := map[string]any{
			"generated_at": friday.Unix(),
			"providers": []any{map[string]any{
				"provider": "claude",
				"accounts": []any{map[string]any{
					"account": "", "label": "test", "is_default": true, "source": "oauth_poll",
					"windows": map[string]any{
						"five_hour": map[string]any{"used_percent": map[string]any{"state": "unavailable", "reason": "not part of this fixture"}, "resets_at": nil, "window_minutes": nil, "is_current": nil},
						"seven_day": map[string]any{"used_percent": map[string]any{"state": "ok", "value": 88.0}, "resets_at": reset.Unix(), "window_minutes": 7 * 24 * 60, "is_current": nil},
						"monthly":   map[string]any{"used_percent": map[string]any{"state": "unavailable", "reason": "extra usage is off"}, "resets_at": nil, "window_minutes": nil, "is_current": nil},
					},
				}},
			}},
		}
		raw, err := json.Marshal(doc)
		if err != nil {
			t.Fatalf("marshal: %v", err)
		}
		s, err := limits.DecodeReport(raw, friday)
		if err != nil {
			t.Fatalf("decode: %v", err)
		}
		return s
	}(t)

	utcCalendar := matrixWithPacing(t, func(p *Pacing) { p.SevenDay.Timezone = "UTC" })
	calendar := Select(utcCalendar, snap, nil, friday, Request{Role: "implementer", Provider: "claude", ForecastDemandBeforeResetPct: floatPtr(0)})
	if calendar.Mode != ModeConserve {
		t.Fatalf("the calendar curve must read Friday-evening 88%% as over the line: %s\n%v", calendar.Mode, calendar.Reason)
	}

	workdays := matrixWithPacing(t, func(p *Pacing) {
		p.SevenDay.Curve = limits.CurveWorkdays
		p.SevenDay.WeekendWeight = 0.25
		p.SevenDay.Timezone = "UTC"
	})
	d := Select(workdays, snap, nil, friday, Request{Role: "implementer", Provider: "claude", ForecastDemandBeforeResetPct: floatPtr(0)})
	if d.Mode == ModeConserve {
		t.Errorf("a week's allowance spent over the WORKING week is on plan under the workdays curve — the curve knob is not reaching the arithmetic: %v", d.Reason)
	}
	if d.Capacity.Pace == nil || d.Capacity.Pace.Curve != limits.CurveWorkdays {
		t.Errorf("the decision must report which curve answered: %+v", d.Capacity.Pace)
	}
}
