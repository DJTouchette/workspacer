package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"github.com/djtouchette/workspacer-hub/internal/routing"
	"github.com/djtouchette/workspacer-hub/internal/usageprefs"
)

// trustedCaller / untrustedCaller mirror how the bus labels a connection: the
// host token is trusted, a scoped pairing token is not.
func trustedCaller() bus.CallerIdentity { return bus.CallerIdentity{Trusted: true} }
func untrustedCaller() bus.CallerIdentity {
	return bus.CallerIdentity{Scope: string(authtoken.ScopeView)}
}

func readSchedule(t *testing.T, h bus.LocalIdentHandler, c bus.CallerIdentity) usagePacingScheduleView {
	t.Helper()
	out, err := h(c, json.RawMessage(`{}`))
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	v, ok := out.(usagePacingScheduleView)
	if !ok {
		t.Fatalf("read answered %T", out)
	}
	return v
}

func TestPacingSchedulePersistsAndIsTrustedOnlyToWrite(t *testing.T) {
	path := filepath.Join(t.TempDir(), "usage-pacing.json")
	prefs, err := usageprefs.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	read := usagePacingSchedule(prefs)
	write := usageSetPacingSchedule(prefs)

	// Default: nobody has chosen, and any tier may ask.
	if got := readSchedule(t, read, untrustedCaller()); got.Schedule != "" || !got.Configurable {
		t.Fatalf("default read = %+v, want unset and configurable", got)
	}

	// The WRITE is host authority only. A view-tier caller is refused, and the
	// refusal must not have written anything.
	if _, err := write(untrustedCaller(), json.RawMessage(`{"schedule":"five_day"}`)); err == nil {
		t.Fatal("a view-tier caller set the pacing schedule")
	} else if !strings.Contains(err.Error(), "usage.setPacingSchedule") {
		t.Errorf("the refusal does not name the method: %v", err)
	}
	if got := readSchedule(t, read, trustedCaller()); got.Schedule != "" {
		t.Fatalf("a refused write stored %q", got.Schedule)
	}

	// A trusted caller may, and the answer is the value the hub now holds.
	out, err := write(trustedCaller(), json.RawMessage(`{"schedule":"five_day"}`))
	if err != nil {
		t.Fatalf("trusted write: %v", err)
	}
	if v := out.(usagePacingScheduleView); v.Schedule != usageprefs.ScheduleFiveDay {
		t.Fatalf("write answered %+v", v)
	}

	// SURVIVES A RESTART: a fresh store over the same path.
	restarted, err := usageprefs.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := readSchedule(t, usagePacingSchedule(restarted), untrustedCaller()); got.Schedule != usageprefs.ScheduleFiveDay {
		t.Errorf("after a hub restart the schedule reads %q", got.Schedule)
	}

	// An invalid value is an ERROR, not a silent fallback, and changes nothing.
	if _, err := write(trustedCaller(), json.RawMessage(`{"schedule":"weekends_only"}`)); err == nil {
		t.Error("an unrecognised schedule was accepted")
	}
	if got := readSchedule(t, read, trustedCaller()); got.Schedule != usageprefs.ScheduleFiveDay {
		t.Errorf("a refused write moved the stored value to %q", got.Schedule)
	}

	// The reader takes no parameters, exactly as usage.report does not.
	if _, err := read(trustedCaller(), json.RawMessage(`{"schedule":"seven_day"}`)); err == nil {
		t.Error("the reader accepted caller parameters")
	}

	// A hub with no preference file says so rather than pretending to save.
	none, _ := usageprefs.Open("")
	if got := readSchedule(t, usagePacingSchedule(none), trustedCaller()); got.Configurable {
		t.Error("a hub with no preference file reports the setting as configurable")
	}
	if _, err := usageSetPacingSchedule(none)(trustedCaller(), json.RawMessage(`{"schedule":"five_day"}`)); err == nil {
		t.Error("a hub with no preference file accepted a save it cannot persist")
	}
}

// TestUsageReportStillTakesNoParametersWithASchedule is the contract this whole
// design exists to preserve: the schedule is hub-side state, so usage.report's
// machine-pinned no-parameter seam is untouched by it.
func TestUsageReportStillTakesNoParametersWithASchedule(t *testing.T) {
	prefs, _ := usageprefs.Open(filepath.Join(t.TempDir(), "usage-pacing.json"))
	if _, err := prefs.Set(usageprefs.ScheduleFiveDay); err != nil {
		t.Fatal(err)
	}
	h := usageReport(routing.New("", nil), newUsageWatcher("http://127.0.0.1:1"), prefs)
	for _, params := range []string{`{"schedule":"seven_day"}`, `{"curve":"calendar"}`, `{"url":"http://other"}`} {
		if _, err := h(trustedCaller(), json.RawMessage(params)); err == nil ||
			!strings.Contains(err.Error(), "no parameters accepted") {
			t.Errorf("usage.report(%s) did not refuse caller parameters: %v", params, err)
		}
	}
	// And the classification that pins it is still the inert one — a method
	// that grew a caller value would have to move out of that map.
	if _, inert := capspec.InertReason("usage.report"); !inert {
		t.Error("usage.report left inertMethods, which is the record that says it takes no caller values")
	}
	if capspec.MissingClassification("usage.pacingSchedule") || capspec.MissingClassification("usage.setPacingSchedule") {
		t.Error("a new hub-native method is unclassified in capspec")
	}
}

// scheduleReportBody builds a claudemon /usage/report with LENGTHS on both
// Anthropic windows, so the projection has a denominator to pace against.
func scheduleReportBody(now time.Time, fiveHourResets, sevenDayResets time.Time, fiveHourPct, sevenDayPct float64) []byte {
	return []byte(fmt.Sprintf(`{
  "generated_at": %d,
  "providers": [{
    "provider": "claude",
    "note": null,
    "accounts": [{
      "account": "", "label": "default", "is_default": true, "source": "oauth_poll",
      "observed_at": %d, "fresh": true, "failure": null,
      "windows": {
        "five_hour": {"used_percent": {"state":"ok","value": %g}, "resets_at": %d, "window_minutes": 300, "is_current": true},
        "seven_day": {"used_percent": {"state":"ok","value": %g}, "resets_at": %d, "window_minutes": 10080, "is_current": true},
        "monthly": {"used_percent": {"state":"unavailable","reason":"not enabled"}, "resets_at": null, "window_minutes": null, "is_current": null}
      }
    }]
  }]
}`, now.Unix(), now.Unix(), fiveHourPct, fiveHourResets.Unix(), sevenDayPct, sevenDayResets.Unix()))
}

// TestTheScheduleReachesTheProjectedReport is the end-to-end assertion the
// fleet's most common bug needs: the stored preference must arrive at the
// NUMBERS a client renders, at a fixed clock, and only for the weekly window.
//
// It runs the real projection (limits.Snapshot.UsageReport) against the real
// installed matrix (routing.New("", nil) — the compiled-in routing.default.yaml)
// with the real preference store between them, which is every layer of
// cmd/hub's handler except its call to time.Now.
func TestTheScheduleReachesTheProjectedReport(t *testing.T) {
	matrix := routing.New("", nil).Matrix()
	if matrix.PaceConfig().Location == nil {
		t.Skip("this host resolved no local timezone, so no weekday curve is evaluable here")
	}
	// A Saturday, inside a window that opened the previous Monday: the whole
	// working week is behind us, so the five-day curve has reached the whole
	// allowance while the calendar one is still four fifths of the way through.
	// This is the instant the two schedules disagree about most.
	loc := matrix.PaceConfig().Location
	now := time.Date(2026, 9, 5, 15, 0, 0, 0, loc)
	if now.Weekday() != time.Saturday {
		t.Fatalf("fixture drift: %s", now.Weekday())
	}
	sevenDayResets := time.Date(2026, 9, 7, 0, 0, 0, 0, loc) // the following Monday
	fiveHourResets := now.Add(90 * time.Minute)

	snap, err := limits.DecodeReport(scheduleReportBody(now, fiveHourResets, sevenDayResets, 40, 82), now)
	if err != nil {
		t.Fatal(err)
	}

	project := func(schedule string) (weekly, short limits.UsagePace) {
		t.Helper()
		dir := t.TempDir()
		prefs, err := usageprefs.Open(filepath.Join(dir, "usage-pacing.json"))
		if err != nil {
			t.Fatal(err)
		}
		if schedule != usageprefs.ScheduleUnset {
			if _, err := prefs.Set(schedule); err != nil {
				t.Fatal(err)
			}
		}
		out := snap.UsageReport(now, prefs.Apply(matrix.PaceConfig()), usageReportValidity)
		acct := out.Providers[0].Accounts[0]
		return acct.Windows[limits.WindowSevenDay].Pace, acct.Windows[limits.WindowFiveHour].Pace
	}

	unsetWeekly, unsetShort := project(usageprefs.ScheduleUnset)
	sevenWeekly, sevenShort := project(usageprefs.ScheduleSevenDay)
	fiveWeekly, fiveShort := project(usageprefs.ScheduleFiveDay)

	// The shipped matrix is `curve: calendar`, so unset and seven_day agree —
	// and that is what "a missing preference preserves existing behaviour"
	// means in numbers.
	if unsetWeekly != sevenWeekly {
		t.Errorf("no preference and an explicit seven_day disagree:\n unset %+v\n seven %+v", unsetWeekly, sevenWeekly)
	}
	if sevenWeekly.Curve != limits.CurveCalendar {
		t.Errorf("seven_day projected the %q curve", sevenWeekly.Curve)
	}
	if fiveWeekly.Curve != limits.CurveFiveDay {
		t.Errorf("five_day projected the %q curve — the stored preference never reached the arithmetic", fiveWeekly.Curve)
	}
	// The whole point, in one number: on a Saturday the five-day curve expects
	// the working week's whole allowance to be gone, and the calendar one does
	// not, so the SAME reading paces differently.
	if !(fiveWeekly.ExpectedPct > sevenWeekly.ExpectedPct) {
		t.Errorf("the five-day curve expects %.2f%% against the calendar's %.2f%% on a Saturday — the schedule changed no value a client renders",
			fiveWeekly.ExpectedPct, sevenWeekly.ExpectedPct)
	}
	if fiveWeekly.UsedPct != sevenWeekly.UsedPct {
		t.Errorf("the schedule moved OBSERVED usage (%.2f vs %.2f); it may only move the expectation",
			fiveWeekly.UsedPct, sevenWeekly.UsedPct)
	}
	// The five-hour window is byte-for-byte identical under every schedule.
	if unsetShort != sevenShort || sevenShort != fiveShort {
		t.Errorf("the five-hour window moved with the weekly schedule:\n unset %+v\n seven %+v\n five  %+v", unsetShort, sevenShort, fiveShort)
	}
	// …and the monthly overage window has no length, so it is absent from the
	// projection's paceable set entirely under both.
	for _, p := range []limits.UsagePace{fiveWeekly, sevenWeekly} {
		if p.Window != limits.WindowSevenDay {
			t.Errorf("weekly pace reports window %q", p.Window)
		}
	}
}

// TestAMidweekResetIsWeightedByRealWeekdays holds the "respect actual rolling
// reset intervals" requirement: nothing may assume a Monday-aligned week.
func TestAMidweekResetIsWeightedByRealWeekdays(t *testing.T) {
	matrix := routing.New("", nil).Matrix()
	loc := matrix.PaceConfig().Location
	if loc == nil {
		t.Skip("no local timezone on this host")
	}
	prefs, _ := usageprefs.Open(filepath.Join(t.TempDir(), "usage-pacing.json"))
	if _, err := prefs.Set(usageprefs.ScheduleFiveDay); err != nil {
		t.Fatal(err)
	}
	cfg := prefs.Apply(matrix.PaceConfig())

	// Reset on a Wednesday afternoon; sample the Thursday inside that window
	// and the Saturday after it. The Saturday reading must not have advanced
	// past the Friday one by more than the Friday hours between them.
	resets := time.Date(2026, 9, 9, 15, 0, 0, 0, loc) // Wednesday
	if resets.Weekday() != time.Wednesday {
		t.Fatalf("fixture drift: %s", resets.Weekday())
	}
	at := func(now time.Time) limits.UsagePace {
		t.Helper()
		snap, err := limits.DecodeReport(scheduleReportBody(now, now.Add(time.Hour), resets, 30, 55), now)
		if err != nil {
			t.Fatal(err)
		}
		return snap.UsageReport(now, cfg, usageReportValidity).
			Providers[0].Accounts[0].Windows[limits.WindowSevenDay].Pace
	}
	fri := at(time.Date(2026, 9, 4, 23, 0, 0, 0, loc))
	sat := at(time.Date(2026, 9, 5, 12, 0, 0, 0, loc))
	sun := at(time.Date(2026, 9, 6, 12, 0, 0, 0, loc))
	for _, p := range []limits.UsagePace{fri, sat, sun} {
		if !p.Known {
			t.Fatalf("a mid-week reset made the weekly pace unknown: %s", p.Because)
		}
		if p.Curve != limits.CurveFiveDay {
			t.Fatalf("curve %q", p.Curve)
		}
	}
	if sat.ExpectedPct != sun.ExpectedPct {
		t.Errorf("the weekend is not flat across a Wednesday-reset window: Sat %.4f%% vs Sun %.4f%%", sat.ExpectedPct, sun.ExpectedPct)
	}
	if sat.ExpectedPct <= fri.ExpectedPct {
		t.Errorf("Friday's last hour must still count: Fri %.4f%% vs Sat %.4f%%", fri.ExpectedPct, sat.ExpectedPct)
	}
	// The next Monday is inside the same window and must resume climbing.
	mon := at(time.Date(2026, 9, 7, 12, 0, 0, 0, loc))
	if mon.ExpectedPct <= sun.ExpectedPct {
		t.Errorf("the curve did not resume on Monday: Sun %.4f%% vs Mon %.4f%%", sun.ExpectedPct, mon.ExpectedPct)
	}
}

// TestNoSchedulePathWritesRoutingYaml is the boundary this design promised not
// to cross, held as a source fact rather than as a claim in a comment: the
// schedule's own handlers write NOTHING, and the store they call writes exactly
// one path — its own, the one it was constructed with.
func TestNoSchedulePathWritesRoutingYaml(t *testing.T) {
	for _, name := range []string{"usageprefs.go", "usagereport.go"} {
		raw, err := os.ReadFile(name)
		if err != nil {
			t.Fatal(err)
		}
		for _, bad := range []string{"routingFile", "os.WriteFile", "os.Create", "os.Rename", "SetMatrix", "routing.DefaultPath"} {
			if strings.Contains(string(raw), bad) {
				t.Errorf("%s mentions %q — a pacing-preference handler must write nothing at all", name, bad)
			}
		}
	}
	// The store touches exactly ONE location: the path it was constructed with.
	// Every filesystem call in the package must name that path (or the temp
	// file and parent directory derived from it) — a literal filename, a
	// config-dir lookup, or anything reaching for the matrix would be a second
	// location, and a second location is how a preference toggle turns into a
	// routing edit.
	raw, err := os.ReadFile(filepath.Join("..", "..", "internal", "usageprefs", "usageprefs.go"))
	if err != nil {
		t.Fatal(err)
	}
	fsCall := regexp.MustCompile(`\bos\.(WriteFile|Rename|Remove|ReadFile|MkdirAll|Create|OpenFile)\(([^)]*)`)
	calls := fsCall.FindAllStringSubmatch(string(raw), -1)
	if len(calls) < 4 {
		t.Fatalf("found only %d filesystem calls in internal/usageprefs — the scan broke and this guard is guarding nothing", len(calls))
	}
	for _, c := range calls {
		args := c[2]
		if !strings.Contains(args, "s.path") && !strings.Contains(args, "tmp") && !strings.Contains(args, "path") {
			t.Errorf("internal/usageprefs calls os.%s(%s) on something other than its own preference path", c[1], args)
		}
	}
}
