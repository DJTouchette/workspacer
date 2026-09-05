package usageprefs

import (
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// matrixConfig is a stand-in for what routing.yaml hands the arithmetic: the
// SHIPPED defaults, plus the two weekend knobs an operator may have moved.
func matrixConfig() limits.PaceConfig {
	return limits.PaceConfig{
		Enabled:               true,
		ConserveAtRatio:       1.25,
		BlockSpendDownAtRatio: 1.0,
		MinElapsedPct:         5,
		Curve:                 limits.CurveCalendar,
		Location:              time.UTC,
		WeekendWeight:         0.5,
		WeekendPolicy:         limits.WeekendSpendTail,
	}
}

func TestAMissingFileIsUnsetAndChangesNothing(t *testing.T) {
	s, err := Open(filepath.Join(t.TempDir(), "nope.json"))
	if err != nil {
		t.Fatalf("an absent preference file is not an error: %v", err)
	}
	if s.Schedule() != ScheduleUnset {
		t.Errorf("absent file reads as %q, want unset", s.Schedule())
	}
	// The whole point of unset: routing.yaml's answer survives untouched,
	// INCLUDING a hand-set workdays curve an operator chose.
	cfg := matrixConfig()
	cfg.Curve, cfg.WeekendPolicy, cfg.WeekendReservePct = limits.CurveWorkdays, limits.WeekendReserve, 15
	if got := s.Apply(cfg); got != cfg {
		t.Errorf("an unset preference modified the matrix config:\n got %+v\nwant %+v", got, cfg)
	}
	// And a nil store is the same answer, so a hub started without the file
	// needs no branch at any reader.
	var nilStore *Store
	if nilStore.Schedule() != ScheduleUnset || nilStore.Apply(cfg) != cfg {
		t.Errorf("a nil store must behave as unset")
	}
	if nilStore.Path() != "" {
		t.Errorf("a nil store claims a path")
	}
}

func TestARoundTripSurvivesAReopen(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "usage-pacing.json")
	s, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	got, err := s.Set("  FIVE_DAY ") // trimmed and folded, because a UI is not a parser
	if err != nil {
		t.Fatalf("set: %v", err)
	}
	if got != ScheduleFiveDay || s.Schedule() != ScheduleFiveDay {
		t.Fatalf("stored %q / in-memory %q", got, s.Schedule())
	}

	// SURVIVES RESTART: a fresh store over the same path reads it back. This is
	// the assertion that makes "stored hub-side" mean anything.
	again, err := Open(path)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	if again.Schedule() != ScheduleFiveDay {
		t.Errorf("after a restart the preference reads %q", again.Schedule())
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if perm := info.Mode().Perm(); perm != 0o600 {
		t.Errorf("preference file mode is %o, want 600 — it lives with the hub's host-trusted state", perm)
	}
	// No temp file left behind by the atomic write.
	if _, err := os.Stat(path + ".tmp"); !os.IsNotExist(err) {
		t.Errorf("the atomic write left its temp file behind")
	}

	// And back again, so the control is genuinely two-way.
	if _, err := again.Set(ScheduleSevenDay); err != nil {
		t.Fatal(err)
	}
	third, _ := Open(path)
	if third.Schedule() != ScheduleSevenDay {
		t.Errorf("the second write did not persist: %q", third.Schedule())
	}
}

func TestAnInvalidPreferenceIsRefusedAndStoresNothing(t *testing.T) {
	path := filepath.Join(t.TempDir(), "usage-pacing.json")
	s, _ := Open(path)
	if _, err := s.Set(ScheduleFiveDay); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"", "weekdays", "5", "five_day_week", "seven"} {
		if _, err := s.Set(bad); err == nil {
			t.Errorf("%q was accepted as a schedule", bad)
		}
	}
	if s.Schedule() != ScheduleFiveDay {
		t.Errorf("a refused write moved the stored value to %q", s.Schedule())
	}
	on, _ := Open(path)
	if on.Schedule() != ScheduleFiveDay {
		t.Errorf("a refused write reached the file: %q", on.Schedule())
	}
}

func TestAnUnreadableFileFallsBackToRoutingYaml(t *testing.T) {
	dir := t.TempDir()
	for _, tc := range []struct{ name, body string }{
		{"malformed.json", "{not json"},
		{"unknown.json", `{"schedule":"fortnight"}`},
		{"empty-value.json", `{"schedule":""}`},
	} {
		path := filepath.Join(dir, tc.name)
		if err := os.WriteFile(path, []byte(tc.body), 0o600); err != nil {
			t.Fatal(err)
		}
		s, err := Open(path)
		if err == nil {
			t.Errorf("%s: a file the hub could not use must be REPORTED, not silently ignored", tc.name)
		}
		if s.Schedule() != ScheduleUnset {
			t.Errorf("%s: read as %q, want unset so routing.yaml answers", tc.name, s.Schedule())
		}
		// Still usable: the operator can fix it from Settings without a restart.
		if _, err := s.Set(ScheduleFiveDay); err != nil {
			t.Errorf("%s: a bad file left the store unwritable: %v", tc.name, err)
		}
	}
}

// TestApplyIsThePrecedenceTable is the contract in the package doc, held as
// code so the three arms cannot drift apart.
func TestApplyIsThePrecedenceTable(t *testing.T) {
	// Start from an operator posture that is NOT the shipped default, so
	// "inherits the matrix" and "overrides the matrix" are distinguishable.
	base := matrixConfig()
	base.Curve = limits.CurveWorkdays
	base.WeekendWeight, base.WeekendPolicy, base.WeekendReservePct = 0.25, limits.WeekendReserve, 20

	if got := ApplySchedule(ScheduleUnset, base); got != base {
		t.Errorf("unset must inherit the matrix verbatim:\n got %+v\nwant %+v", got, base)
	}

	seven := ApplySchedule(ScheduleSevenDay, base)
	if seven.Curve != limits.CurveCalendar {
		t.Errorf("an explicit seven_day must mean the CALENDAR shape, got %q", seven.Curve)
	}
	// …and must move nothing else.
	want := base
	want.Curve = limits.CurveCalendar
	if seven != want {
		t.Errorf("seven_day changed a field other than the curve:\n got %+v\nwant %+v", seven, want)
	}

	five := ApplySchedule(ScheduleFiveDay, base)
	if five.Curve != limits.CurveFiveDay {
		t.Errorf("five_day must select the five_day curve, got %q", five.Curve)
	}
	if five.WeekendWeight != 0 || five.WeekendPolicy != limits.WeekendSpendTail || five.WeekendReservePct != 0 {
		t.Errorf("five_day must neutralize the weekend knobs (weight %g, policy %q, reserve %g)",
			five.WeekendWeight, five.WeekendPolicy, five.WeekendReservePct)
	}
	// Everything the schedule has no opinion about is still the matrix's.
	if five.Enabled != base.Enabled || five.ConserveAtRatio != base.ConserveAtRatio ||
		five.BlockSpendDownAtRatio != base.BlockSpendDownAtRatio ||
		five.MinElapsedPct != base.MinElapsedPct || five.ExpectedOffsetPct != base.ExpectedOffsetPct ||
		five.Location != base.Location {
		t.Errorf("five_day moved a band, a bootstrap value or the timezone:\n got %+v\nfrom %+v", five, base)
	}
	// A disabled matrix stays disabled: the schedule is a shape, not a switch.
	off := base
	off.Enabled = false
	if ApplySchedule(ScheduleFiveDay, off).Enabled {
		t.Errorf("five_day re-enabled pacing that routing.yaml switched off")
	}
}

// TestConcurrentReadersAndWriters is the state-safety claim: the store is read
// on every usage.report and written from a Settings click, and those races are
// real on a machine with the Overview pane open.
func TestConcurrentReadersAndWriters(t *testing.T) {
	s, _ := Open(filepath.Join(t.TempDir(), "usage-pacing.json"))
	var wg sync.WaitGroup
	for i := range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			v := ScheduleFiveDay
			if i%2 == 0 {
				v = ScheduleSevenDay
			}
			for range 25 {
				if _, err := s.Set(v); err != nil {
					t.Errorf("set: %v", err)
					return
				}
			}
		}()
	}
	for range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for range 25 {
				switch got := s.Schedule(); got {
				case ScheduleFiveDay, ScheduleSevenDay, ScheduleUnset:
				default:
					t.Errorf("read a torn value %q", got)
					return
				}
			}
		}()
	}
	wg.Wait()
	if _, err := Validate(s.Schedule()); err != nil {
		t.Errorf("the settled value is not a schedule: %v", err)
	}
}
