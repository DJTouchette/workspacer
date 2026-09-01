package routing

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// now is the instant every case below decides at. Everything else is expressed
// relative to it, because the one thing this layer must never do is read a
// clock of its own.
var policyNow = time.Unix(1788126404, 0)

func shipped(t *testing.T) *Matrix {
	t.Helper()
	m, err := Load("", nil)
	if err != nil {
		t.Fatalf("load the shipped defaults: %v", err)
	}
	return m
}

// snapshotOf builds a usage document out of a compact per-window spec and
// decodes it through the real DecodeReport, so the tests exercise the same wire
// path a live hub does.
//
//	used < 0  -> the window reports `unavailable` (copilot's 403)
//	resets    -> offset from policyNow. NEGATIVE is a window that has CLOSED.
type winSpec struct {
	used   float64
	resets time.Duration
	// noReset omits resets_at entirely.
	noReset bool
	// minutes is the window's declared LENGTH. Zero means the source reported
	// none, which is what every case written before pacing existed says — and
	// a window with no length can never be paced, which is why those cases are
	// unaffected by the pacing block.
	minutes int64
}

func snapshotOf(t *testing.T, provider, account string, windows map[string]winSpec) limits.Snapshot {
	t.Helper()
	return snapshotOfProviders(t, account, map[string]map[string]winSpec{provider: windows})
}

// snapshotOfProviders is snapshotOf for more than one provider at a time, which
// is what a CROSS-PROVIDER mode shift needs: the mode comes from one provider's
// document row and the shift's landing capacity comes from another's.
func snapshotOfProviders(t *testing.T, account string, byProvider map[string]map[string]winSpec) limits.Snapshot {
	t.Helper()
	wire := func(w winSpec) map[string]any {
		out := map[string]any{"window_minutes": nil, "is_current": nil}
		if w.minutes > 0 {
			out["window_minutes"] = w.minutes
		}
		if w.used < 0 {
			out["used_percent"] = map[string]any{"state": "unavailable", "reason": "this provider will never publish a window"}
		} else {
			out["used_percent"] = map[string]any{"state": "ok", "value": w.used}
		}
		if w.noReset {
			out["resets_at"] = nil
		} else {
			out["resets_at"] = policyNow.Add(w.resets).Unix()
		}
		return out
	}
	names := make([]string, 0, len(byProvider))
	for name := range byProvider {
		names = append(names, name)
	}
	sort.Strings(names)
	providers := make([]any, 0, len(names))
	for _, provider := range names {
		windows := byProvider[provider]
		byName := map[string]any{}
		for _, name := range limits.WindowOrder {
			spec, ok := windows[name]
			if !ok {
				// Absent from the SPEC still means present on the wire and
				// permanently unavailable, which is how a provider says "I do
				// not have this window" — the real document always carries all
				// three keys.
				spec = winSpec{used: -1, noReset: true}
			}
			byName[name] = wire(spec)
		}
		providers = append(providers, map[string]any{
			"provider": provider,
			"accounts": []any{map[string]any{
				"account": account, "label": "test", "is_default": true,
				"source": "oauth_poll", "windows": byName,
			}},
		})
	}
	doc := map[string]any{
		"generated_at": policyNow.Unix(),
		"providers":    providers,
	}
	raw, err := json.Marshal(doc)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	snap, err := limits.DecodeReport(raw, policyNow)
	if err != nil {
		t.Fatalf("decode: %v", err)
	}
	return snap
}

// ---------------------------------------------------------------------------
// THE MODE TABLE
// ---------------------------------------------------------------------------

func TestDecideModeTable(t *testing.T) {
	m := shipped(t)
	pct := func(v float64) *float64 { return &v }

	for _, tc := range []struct {
		name    string
		windows map[string]winSpec
		// forecast is nil for "the caller said nothing".
		forecast *float64
		manual   string
		want     Mode
		why      string
	}{
		{
			name:     "healthy, a reset far away, nothing forecast",
			windows:  map[string]winSpec{"five_hour": {used: 12, resets: 4 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeNormal,
			why:      "the reset is outside the 90-minute window, so there is nothing expiring to spend",
		},
		{
			name:     "every spend-down arm holds",
			windows:  map[string]winSpec{"five_hour": {used: 12, resets: 75 * time.Minute}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeSpendDown,
			why:      "reset in 75m (< 90), 88% remaining (>= 50), demand 0% (< 30% of 88%)",
		},
		{
			name:     "THE DEFECT: a window that closed an hour ago must not spend down",
			windows:  map[string]winSpec{"five_hour": {used: 20, resets: -time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeNormal,
			why:      "time_to_reset is -3600s, which is 'under 90 minutes' on any unguarded comparison, and 20% used reads as 80% remaining. Both spend-down preconditions LOOK satisfied and the window has been gone for an hour",
		},
		{
			name:     "THE OTHER DEFECT: a stale high percentage must not conserve",
			windows:  map[string]winSpec{"five_hour": {used: 95, resets: -47 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeNormal,
			why:      "the live codex row's shape. 95% used off a window that closed two days ago is history; conserving on it is self-reinforcing because conserving slows the arrival of a fresh reading",
		},
		{
			name:     "a genuinely scarce window conserves",
			windows:  map[string]winSpec{"five_hour": {used: 94, resets: 2 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeConserve,
			why:      "94% used on a RUNNING window is over the matrix's red_at_used_pct of 90",
		},
		{
			name:     "§33: a healthy short window does not license spending against a constrained weekly one",
			windows:  map[string]winSpec{"five_hour": {used: 30, resets: 40 * time.Minute}, "seven_day": {used: 85, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeNormal,
			why:      "the spec's own example: 5h resets in 40 minutes with room, and the weekly constraint still makes the model scarce",
		},
		{
			name:     "demand above what is left conserves",
			windows:  map[string]winSpec{"five_hour": {used: 40, resets: 3 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(75),
			want:     ModeConserve,
			why:      "75% of the allowance is forecast and 60% is left",
		},
		{
			name:     "an unknown forecast cannot spend down",
			windows:  map[string]winSpec{"five_hour": {used: 12, resets: 75 * time.Minute}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: nil,
			want:     ModeNormal,
			why:      "every other arm holds. Spending down on a forecast nobody supplied is the arbitrary token consumption Invariant 6 forbids",
		},
		{
			name:     "demand too large a share of what remains does not spend down",
			windows:  map[string]winSpec{"five_hour": {used: 12, resets: 75 * time.Minute}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(40),
			want:     ModeNormal,
			why:      "40% is not below 30% of the 88% remaining (26.4%)",
		},
		{
			name:     "too little left to spend down",
			windows:  map[string]winSpec{"five_hour": {used: 60, resets: 75 * time.Minute}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeNormal,
			why:      "40% remaining is under the matrix's min_remaining_pct of 50",
		},
		{
			name:     "an exhausted allowance conserves",
			windows:  map[string]winSpec{"five_hour": {used: 100, resets: 2 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			want:     ModeConserve,
			why:      "",
		},
		{
			name:     "nothing readable at all stays NORMAL",
			windows:  map[string]winSpec{"five_hour": {used: 67, noReset: true}, "seven_day": {used: 11, noReset: true}},
			forecast: pct(0),
			want:     ModeNormal,
			why:      "the matrix's when_unknown for codex is `yellow`, and yellow is not RED — an unreadable provider is not a constrained one",
		},
		{
			name:     "the manual override wins over a healthy reading",
			windows:  map[string]winSpec{"five_hour": {used: 12, resets: 75 * time.Minute}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			manual:   "conserve",
			want:     ModeConserve,
			why:      "'Conserve Claude for the next few hours' is the user speaking; a threshold that overruled it would make the control a suggestion",
		},
		{
			name:     "the manual override wins over a scarce reading too",
			windows:  map[string]winSpec{"five_hour": {used: 99, resets: 2 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
			forecast: pct(0),
			manual:   "spend_down",
			want:     ModeSpendDown,
			why:      "it cuts both ways, and 'Critical production bug, ignore conservation' is the spec's own example",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			snap := snapshotOf(t, "codex", "acct", tc.windows)
			cap := ReadCapacity(m, snap, "codex", "acct", policyNow)
			demand := limits.Forecast(tc.forecast, nil, m.ForecastWeights)
			manual := tc.manual
			if manual == "" {
				manual = m.ModeFor("codex")
			}
			got := DecideMode(cap, demand, m.Thresholds, manual)
			if got.Mode != tc.want {
				t.Errorf("mode = %q, want %q\n  why: %s\n  capacity: %s\n  reasons: %s",
					got.Mode, tc.want, tc.why, cap.Because, strings.Join(got.Reason, " | "))
			}
			if len(got.Reason) == 0 {
				t.Error("a mode with no reasons is Invariant 5 unmet")
			}
			if got.Manual != (tc.manual != "" && tc.manual != "auto") {
				t.Errorf("Manual = %v for manual=%q", got.Manual, tc.manual)
			}
		})
	}
}

// TestTheThresholdsComeFromTheFile is the "prove the value arrives" check. It
// moves each number in routing.yaml and requires the verdict to move with it —
// a threshold that is loaded but never compared against is this fleet's most
// common bug, and it looks identical to one that works.
//
// EVERY NUMBER UNDER `thresholds:` HAS ITS OWN CASE, and the two health bands
// have one EACH rather than one between them. The earlier version moved yellow
// and red together and asserted a red-only outcome, which a hardcoded yellow
// would have passed unchanged, and it never touched
// max_forecast_pct_of_remaining at all. A test that claims coverage it does not
// have is worse than an obviously narrow one, because it is the reason nobody
// looks again.
func TestTheThresholdsComeFromTheFile(t *testing.T) {
	base := map[string]winSpec{
		"five_hour": {used: 12, resets: 75 * time.Minute},
		"seven_day": {used: 11, resets: 96 * time.Hour},
	}
	// yellowBase is 75% used on a RUNNING window: over the shipped
	// yellow_at_used_pct of 70 and under the shipped red_at_used_pct of 90. It
	// is what tells the two bands apart, because Bands.Valid refuses an
	// inverted pair — red cannot be dropped below yellow to isolate it, so red
	// has to be RAISED to meet a reading yellow already covers.
	yellowBase := map[string]winSpec{
		"five_hour": {used: 75, resets: 3 * time.Hour},
		"seven_day": {used: 11, resets: 96 * time.Hour},
	}
	zero := 0.0

	modeAt := func(t *testing.T, userYAML string, windows map[string]winSpec, forecast *float64) (Mode, Capacity) {
		t.Helper()
		m, err := Load("test.yaml", []byte(userYAML))
		if err != nil {
			t.Fatalf("load: %v", err)
		}
		snap := snapshotOf(t, "codex", "acct", windows)
		cap := ReadCapacity(m, snap, "codex", "acct", policyNow)
		v := DecideMode(cap, limits.Forecast(forecast, nil, m.ForecastWeights), m.Thresholds, m.ModeFor("codex"))
		return v.Mode, cap
	}
	mode := func(t *testing.T, userYAML string, windows map[string]winSpec) (Mode, Capacity) {
		t.Helper()
		return modeAt(t, userYAML, windows, &zero)
	}

	if got, _ := mode(t, "", base); got != ModeSpendDown {
		t.Fatalf("the shipped thresholds do not spend down on the control case (%q) — the rest of this test proves nothing", got)
	}
	if got, cap := mode(t, "", yellowBase); got != ModeNormal || cap.Health != limits.HealthYellow {
		t.Fatalf("the yellow control case is %q/%q, want normal/yellow — the band cases below prove nothing", got, cap.Health)
	}

	// time_to_reset_minutes: shrink the window below 75 and the same reading
	// must stop spending down.
	if got, _ := mode(t, "thresholds:\n  spend_down:\n    time_to_reset_minutes: 30\n", base); got != ModeNormal {
		t.Errorf("time_to_reset_minutes: 30 still spent down on a 75-minute reset (%q) — the file's window is not reaching the comparison", got)
	}
	// min_remaining_pct: raise the floor above 88 and the same reading must
	// stop spending down.
	if got, _ := mode(t, "thresholds:\n  spend_down:\n    min_remaining_pct: 95\n", base); got != ModeNormal {
		t.Errorf("min_remaining_pct: 95 still spent down on 88%% remaining (%q)", got)
	}
	// max_forecast_pct_of_remaining: the third spend-down arm, and the one the
	// review found unproven. 20% forecast is under 30% of the 88% remaining
	// (26.4%) and spends down; move ONLY that number to 20 and the budget falls
	// to 17.6%, so the identical reading and the identical forecast must stop.
	// Deliberately NOT tested by setting it to 0 against a 0% forecast — that
	// arm would pass against a hardcoded 0 just as happily.
	twenty := 20.0
	if got, _ := modeAt(t, "", base, &twenty); got != ModeSpendDown {
		t.Fatalf("the shipped max_forecast_pct_of_remaining does not spend down at 20%% forecast (%q) — the case below proves nothing", got)
	}
	if got, _ := modeAt(t, "thresholds:\n  spend_down:\n    max_forecast_pct_of_remaining: 20\n", base, &twenty); got != ModeNormal {
		t.Errorf("max_forecast_pct_of_remaining: 20 still spent down on a 20%% forecast against 88%% remaining (%q) — the file's budget is not reaching the comparison", got)
	}
	// yellow_at_used_pct ALONE. red stays at the shipped 90, so nothing here
	// can be explained by red: the only thing that moves is where GREEN stops,
	// and spend-down requires GREEN (§33).
	if got, cap := mode(t, "thresholds:\n  health:\n    yellow_at_used_pct: 10\n", base); got != ModeNormal || cap.Health != limits.HealthYellow {
		t.Errorf("yellow_at_used_pct: 10 left a 12%%-used window at %q (mode %q) — the file's YELLOW band is not reaching the ladder, and red is untouched here so red cannot be what carried the earlier case", cap.Health, got)
	}
	// red_at_used_pct ALONE, on the 75%-used window that is already YELLOW
	// under the shipped bands. yellow stays at 70, so only red moves.
	if got, cap := mode(t, "thresholds:\n  health:\n    red_at_used_pct: 72\n", yellowBase); got != ModeConserve || cap.Health != limits.HealthRed {
		t.Errorf("red_at_used_pct: 72 left a 75%%-used window at %q (mode %q) — the file's RED band is not reaching the ladder", cap.Health, got)
	}
	// And both together, which is the case that was here before.
	if got, cap := mode(t, "thresholds:\n  health:\n    yellow_at_used_pct: 5\n    red_at_used_pct: 10\n", base); got != ModeConserve {
		t.Errorf("red_at_used_pct: 10 did not make a 12%%-used window RED (%q, health %q) — the file's bands are not reaching the ladder", got, cap.Health)
	}
	// modes.providers: the manual override is read per provider.
	if got, _ := mode(t, "modes:\n  providers:\n    codex: conserve\n", base); got != ModeConserve {
		t.Errorf("modes.providers.codex: conserve was ignored (%q)", got)
	}
	// The spec's vendor alias must reach the same place.
	if got, _ := mode(t, "modes:\n  providers:\n    openai: conserve\n", base); got != ModeConserve {
		t.Errorf("modes.providers.openai (the spec's name for codex) was ignored (%q)", got)
	}
	// providers.when_unknown: an unreadable provider follows the FILE.
	dark := map[string]winSpec{"five_hour": {used: 67, noReset: true}, "seven_day": {used: 11, noReset: true}}
	if got, cap := mode(t, "providers:\n  codex:\n    when_unknown: red\n", dark); got != ModeConserve {
		t.Errorf("when_unknown: red on a dark provider did not conserve (%q); observed health was %q and assumed %q", got, cap.Health, cap.AssumedHealth)
	}
	if _, cap := mode(t, "providers:\n  codex:\n    when_unknown: red\n", dark); cap.Health != limits.HealthUnknown {
		t.Errorf("the OBSERVED health became %q — when_unknown is policy, and reporting it as evidence is how 'we could not read codex' turns into a fact", cap.Health)
	}
}

// ---------------------------------------------------------------------------
// SELECTION
// ---------------------------------------------------------------------------

func TestSelectResolvesRolesAndHonoursTheProviderConstraint(t *testing.T) {
	m := shipped(t)
	zero := 0.0
	healthy := snapshotOf(t, "codex", "acct", map[string]winSpec{
		"five_hour": {used: 12, resets: 4 * time.Hour},
		"seven_day": {used: 11, resets: 96 * time.Hour},
	})

	t.Run("a role resolves to the active profile's model", func(t *testing.T) {
		d := Select(m, healthy, nil, policyNow, Request{Role: "scout", ForecastDemandBeforeResetPct: &zero})
		if !d.Eligible || d.Provider != "codex" || d.Model != "gpt-5.6-terra" || d.Capability != "balanced" {
			t.Fatalf("got %+v", d)
		}
	})

	t.Run("a provider the matrix cannot serve is REFUSED, not substituted", func(t *testing.T) {
		d := Select(m, healthy, nil, policyNow, Request{Role: "scout", Provider: "copilot", ForecastDemandBeforeResetPct: &zero})
		if d.Eligible {
			t.Fatalf("copilot answered with %s %s — §32 is explicit that critical work must never silently land on a provider the caller did not ask for", d.Provider, d.Model)
		}
		if d.Model != "" {
			t.Errorf("a refusal carried a model: %q", d.Model)
		}
		if !strings.Contains(strings.Join(d.Reason, " "), "copilot") {
			t.Errorf("the refusal does not name the provider it refused: %v", d.Reason)
		}
	})

	t.Run("a provider another profile serves is taken from that profile", func(t *testing.T) {
		d := Select(m, healthy, nil, policyNow, Request{Role: "implementer", Provider: "claude", ForecastDemandBeforeResetPct: &zero})
		if !d.Eligible || d.Provider != "claude" {
			t.Fatalf("got %+v", d)
		}
		if d.Model != "opus" {
			t.Errorf("model = %q, want opus — anthropic_only puts frontier on claude", d.Model)
		}
	})

	t.Run("the spec's vendor names are accepted on the wire", func(t *testing.T) {
		d := Select(m, healthy, nil, policyNow, Request{Role: "scout", Provider: "openai", ForecastDemandBeforeResetPct: &zero})
		if !d.Eligible || d.Provider != "codex" {
			t.Fatalf("openai did not normalize to codex: %+v", d)
		}
	})

	t.Run("a cross-family reviewer is independent, and says so", func(t *testing.T) {
		d := Select(m, healthy, nil, policyNow, Request{
			Role: "reviewer", PreviousProvider: "codex", RequireIndependentFamily: true,
			ForecastDemandBeforeResetPct: &zero,
		})
		if !d.Eligible || d.Provider != "claude" {
			t.Fatalf("got %+v", d)
		}
		if !d.IndependentFamily {
			t.Error("a claude reviewer after a codex implementer is a different family")
		}
	})

	t.Run("independence that cannot be arranged is REPORTED, never silent", func(t *testing.T) {
		d := Select(m, healthy, nil, policyNow, Request{
			Role: "reviewer", Profile: "codex_only", Provider: "codex",
			PreviousProvider: "codex", RequireIndependentFamily: true,
			ForecastDemandBeforeResetPct: &zero,
		})
		if !d.Eligible {
			t.Fatalf("codex_only must still answer: %+v", d)
		}
		if d.IndependentFamily {
			t.Error("codex reviewing codex is not an independent family")
		}
		joined := strings.Join(d.Reason, " ")
		if !strings.Contains(joined, "independent family was REQUIRED") {
			t.Errorf("the requirement was dropped in silence: %v", d.Reason)
		}
		if !d.Fresh {
			t.Error("codex_only's reviewer must be `fresh` — a Sol reviewer inheriting the Sol implementer's reasoning is not a reviewer")
		}
	})

	t.Run("an unknown role is refused rather than guessed", func(t *testing.T) {
		d := Select(m, healthy, nil, policyNow, Request{Role: "wizard", ForecastDemandBeforeResetPct: &zero})
		if d.Eligible {
			t.Fatalf("an unknown role produced a model: %+v", d)
		}
	})

	t.Run("a usage report that cannot be read still answers, with UNKNOWN capacity", func(t *testing.T) {
		d := Select(m, limits.Snapshot{}, fmt.Errorf("connection refused"), policyNow, Request{
			Role: "scout", ForecastDemandBeforeResetPct: &zero,
		})
		if !d.Eligible || d.Model == "" {
			t.Fatalf("a hub that refuses to route because claudemon is restarting is worse than one that routes conservatively: %+v", d)
		}
		if d.Capacity.Health != limits.HealthUnknown {
			t.Errorf("health = %q, want unknown", d.Capacity.Health)
		}
		if d.Mode != ModeNormal {
			t.Errorf("mode = %q on an unreadable report, want normal", d.Mode)
		}
		if !strings.Contains(d.Capacity.Because, "connection refused") {
			t.Errorf("the failure is not named: %q", d.Capacity.Because)
		}
	})
}

// TestTheModeMovesTheCapability proves mode_shifts reaches the selection —
// otherwise the mode is a label on a decision it did not change.
func TestTheModeMovesTheCapability(t *testing.T) {
	m := shipped(t)
	zero := 0.0

	spendDown := snapshotOf(t, "codex", "acct", map[string]winSpec{
		"five_hour": {used: 12, resets: 75 * time.Minute},
		"seven_day": {used: 11, resets: 96 * time.Hour},
	})
	d := Select(m, spendDown, nil, policyNow, Request{Role: "scout", Provider: "codex", ForecastDemandBeforeResetPct: &zero})
	if d.Mode != ModeSpendDown {
		t.Fatalf("mode = %q, want spend_down — the rest of this test proves nothing", d.Mode)
	}
	if d.BaseCapability != "balanced" || d.Capability != "frontier" {
		t.Errorf("capability %q -> %q, want balanced -> frontier: §13's 'balanced scout -> frontier scout'", d.BaseCapability, d.Capability)
	}
	if d.Model != "gpt-5.6-sol" {
		t.Errorf("model = %q, want gpt-5.6-sol (Terra promoted to Sol)", d.Model)
	}

	conserve := snapshotOf(t, "codex", "acct", map[string]winSpec{
		"five_hour": {used: 95, resets: 2 * time.Hour},
		"seven_day": {used: 11, resets: 96 * time.Hour},
	})
	d = Select(m, conserve, nil, policyNow, Request{Role: "scout", Provider: "codex", ForecastDemandBeforeResetPct: &zero})
	if d.Mode != ModeConserve {
		t.Fatalf("mode = %q, want conserve", d.Mode)
	}
	if d.Capability != "cheap" || d.Model != "gpt-5.6-luna" {
		t.Errorf("capability %q model %q, want cheap / gpt-5.6-luna: §12's 'scouting -> cheaper tier'", d.Capability, d.Model)
	}
	// §12 is explicit that implementation STAYS frontier while scouting drops.
	// A capability->capability map would have moved both.
	impl := Select(m, conserve, nil, policyNow, Request{Role: "implementer", Provider: "codex", ForecastDemandBeforeResetPct: &zero})
	if impl.Capability != "frontier" {
		t.Errorf("the implementer moved to %q under conserve — §12 keeps frontier for implementation and hard diagnosis", impl.Capability)
	}

	// THE FIXER STAYS BALANCED UNDER CONSERVE. §12's behaviour table reads
	// "routine fixes -> balanced tier", and the shipped matrix demoted it to
	// `cheap` while its own comment claimed otherwise. The spec is
	// authoritative and the comment agreed with it, so the DATA moved: there is
	// no `conserve.fixer` entry any more, and its absence is what keeps the
	// role on `balanced`. Demoting the role that repairs broken work is also
	// the demotion most likely to cost more than it saves.
	fix := Select(m, conserve, nil, policyNow, Request{Role: "fixer", Provider: "codex", ForecastDemandBeforeResetPct: &zero})
	if fix.Mode != ModeConserve {
		t.Fatalf("the fixer case is not in conserve (%q) — it proves nothing", fix.Mode)
	}
	if fix.BaseCapability != "balanced" || fix.Capability != "balanced" {
		t.Errorf("fixer capability %q -> %q under conserve, want balanced -> balanced: §12 keeps routine fixes on the balanced tier",
			fix.BaseCapability, fix.Capability)
	}
	if fix.Model != "gpt-5.6-terra" {
		t.Errorf("fixer model = %q under conserve, want gpt-5.6-terra", fix.Model)
	}
	if _, shifted := m.ShiftFor("conserve", "fixer"); shifted {
		t.Error("the shipped matrix carries a conserve shift for `fixer` again — §12 says routine fixes stay balanced, and a shift entry here is the contradiction the review found")
	}
	// And the shift is still DATA, not a hardcoded exemption: a user who
	// disagrees with §12 can still demote their own fixer.
	demoted, err := Load("test.yaml", []byte("mode_shifts:\n  conserve:\n    fixer: cheap\n"))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if got := Select(demoted, conserve, nil, policyNow, Request{Role: "fixer", Provider: "codex", ForecastDemandBeforeResetPct: &zero}); got.Capability != "cheap" {
		t.Errorf("a user matrix that DOES demote the fixer was ignored (capability %q) — the default moved, not the mechanism", got.Capability)
	}

	// And the shift is the FILE's, not Go's.
	tuned, err := Load("test.yaml", []byte("mode_shifts:\n  conserve:\n    scout: frontier\n"))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	d = Select(tuned, conserve, nil, policyNow, Request{Role: "scout", Provider: "codex", ForecastDemandBeforeResetPct: &zero})
	if d.Capability != "frontier" {
		t.Errorf("an edited mode_shifts block did not reach the selection: capability = %q", d.Capability)
	}
}

// TestADecisionAlwaysExplainsItself is Invariant 5, over every shape a decision
// can take.
func TestADecisionAlwaysExplainsItself(t *testing.T) {
	m := shipped(t)
	zero := 0.0
	for _, tc := range []struct {
		name string
		req  Request
		snap limits.Snapshot
	}{
		{"an ordinary answer", Request{Role: "scout", ForecastDemandBeforeResetPct: &zero}, snapshotOf(t, "codex", "acct", map[string]winSpec{"five_hour": {used: 12, resets: 4 * time.Hour}})},
		{"a refusal", Request{Role: "scout", Provider: "opencode", ForecastDemandBeforeResetPct: &zero}, limits.Snapshot{}},
		{"an unknown role", Request{Role: "wizard"}, limits.Snapshot{}},
		{"an unknown profile", Request{Role: "scout", Profile: "does-not-exist"}, limits.Snapshot{}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			d := Select(m, tc.snap, nil, policyNow, tc.req)
			if len(d.Reason) == 0 {
				t.Fatal("no reasons at all — a routing system nobody can explain will eventually seem haunted")
			}
			for i, r := range d.Reason {
				if strings.TrimSpace(r) == "" {
					t.Errorf("reason[%d] is empty", i)
				}
			}
			if d.DecidedAt != policyNow.Unix() {
				t.Errorf("DecidedAt = %d, want %d — the decision must be stamped with the clock it judged against", d.DecidedAt, policyNow.Unix())
			}
		})
	}
}

// TestNoNilMatrixPanic — a hub whose matrix failed to load at all must answer,
// not crash the bus connection.
func TestNoNilMatrixPanic(t *testing.T) {
	d := Select(nil, limits.Snapshot{}, nil, policyNow, Request{Role: "scout"})
	if d.Eligible || len(d.Reason) == 0 {
		t.Fatalf("got %+v", d)
	}
}

// TestThePolicyLayerNeverReadsTheRawScalar is the source guard behind
// limits.Bucket.DisplayOnlyRawUsedPercent's name.
//
// The P0 review's finding was that the raw ungated percentage — a real figure
// off a window that may have closed days ago — sat on the bucket as an exported
// FIELD, one obvious-looking member away from any policy code that wanted a
// number. It is now a long, unmistakable accessor, and this test is what keeps
// it out of the package that must never call it: the decision surface is
// Bucket.Reading, which refuses a percentage off a closed window.
func TestThePolicyLayerNeverReadsTheRawScalar(t *testing.T) {
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatalf("read the package directory: %v", err)
	}
	scanned := 0
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".go") || strings.HasSuffix(e.Name(), "_test.go") {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(".", e.Name()))
		if err != nil {
			t.Fatalf("read %s: %v", e.Name(), err)
		}
		scanned++
		if strings.Contains(string(raw), "DisplayOnlyRawUsedPercent") {
			t.Errorf("%s reaches the UNGATED raw usage scalar. That value can describe a window that closed two days ago — reading it here is the phantom-CONSERVE defect arriving through a second door. Use Bucket.Reading, which refuses a percentage off a closed window; the raw figure is for display and explanation, and internal/limits already folds it into BucketReport.Explain as PROSE.", e.Name())
		}
	}
	if scanned < 3 {
		t.Fatalf("scanned only %d source files — the package was renamed or moved and this guard is guarding nothing", scanned)
	}
}

// TestADisabledProviderIsNotSELECTED is the other half of
// TestDisablingIsExplicit, which proved only that `providers.codex.enabled:
// false` PARSES.
//
// routing.yaml tells the operator, in its own header, that deletion cannot
// disable anything and that `enabled: false` is the one spelling that can. The
// selection path honoured that on a per-capability ASSIGNMENT and ignored it on
// the PROVIDER, so an operator could take codex out of service, watch the flag
// load, and still be handed codex — a setting written and never read, which is
// indistinguishable from one that works right up until it matters.
func TestADisabledProviderIsNotSelected(t *testing.T) {
	zero := 0.0
	healthy := snapshotOfProviders(t, "acct", map[string]map[string]winSpec{
		"codex":  {"five_hour": {used: 12, resets: 4 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
		"claude": {"five_hour": {used: 12, resets: 4 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}},
	})
	off := func(t *testing.T, provider string) *Matrix {
		t.Helper()
		m, err := Load("test.yaml", []byte("providers:\n  "+provider+":\n    enabled: false\n"))
		if err != nil {
			t.Fatalf("load: %v", err)
		}
		if pol, ok := m.ProviderPolicy(provider); !ok || pol.IsEnabled() {
			t.Fatalf("the file's providers.%s.enabled: false did not survive the load — the rest proves nothing", provider)
		}
		return m
	}

	t.Run("the role's own provider is refused, not substituted", func(t *testing.T) {
		// scout -> balanced -> codex under `mixed`, with no provider named by
		// the caller at all.
		d := Select(off(t, "codex"), healthy, nil, policyNow, Request{Role: "scout", ForecastDemandBeforeResetPct: &zero})
		if d.Eligible {
			t.Fatalf("a disabled codex still answered with %s %s", d.Provider, d.Model)
		}
		if d.Model != "" {
			t.Errorf("a refusal carried a model: %q", d.Model)
		}
		if !strings.Contains(strings.Join(d.Reason, " "), "enabled: false") {
			t.Errorf("the refusal does not name the flag that caused it: %v", d.Reason)
		}
	})

	t.Run("a provider asked about by name is refused", func(t *testing.T) {
		d := Select(off(t, "codex"), healthy, nil, policyNow, Request{Role: "scout", Provider: "codex", ForecastDemandBeforeResetPct: &zero})
		if d.Eligible {
			t.Fatalf("got %+v", d)
		}
	})

	t.Run("the spec's vendor alias disables the same provider", func(t *testing.T) {
		m, err := Load("test.yaml", []byte("providers:\n  openai:\n    enabled: false\n"))
		if err != nil {
			t.Fatalf("load: %v", err)
		}
		if d := Select(m, healthy, nil, policyNow, Request{Role: "scout", ForecastDemandBeforeResetPct: &zero}); d.Eligible {
			t.Errorf("providers.openai.enabled: false did not take codex out of service: %+v", d)
		}
	})

	t.Run("another profile is not a way around it", func(t *testing.T) {
		// implementer -> frontier -> claude under `anthropic_only`, which
		// assignmentFor will reach for when the caller names claude. A disabled
		// provider must not be reachable through a cross-profile search either.
		d := Select(off(t, "claude"), healthy, nil, policyNow, Request{Role: "implementer", Provider: "claude", ForecastDemandBeforeResetPct: &zero})
		if d.Eligible {
			t.Fatalf("a disabled claude was still reachable through another profile: %s %s", d.Provider, d.Model)
		}
	})

	t.Run("disabling one provider does not disable the others", func(t *testing.T) {
		// The control. A refusal that refused everything would pass every
		// assertion above and be a far worse bug than the one being fixed.
		d := Select(off(t, "codex"), healthy, nil, policyNow, Request{Role: "reviewer", ForecastDemandBeforeResetPct: &zero})
		if !d.Eligible || d.Provider != "claude" || d.Model != "sonnet" {
			t.Fatalf("disabling codex took claude down with it: %+v", d)
		}
	})

	t.Run("the shipped matrix disables nothing", func(t *testing.T) {
		d := Select(shipped(t), healthy, nil, policyNow, Request{Role: "scout", ForecastDemandBeforeResetPct: &zero})
		if !d.Eligible {
			t.Fatalf("the shipped defaults refuse an ordinary scout: %+v", d)
		}
	})
}

// TestACrossProviderShiftIsCheckedAgainstTheProviderItLandsOn is the second
// half of the mode-shift story.
//
// CROSSING PROVIDERS IS ALLOWED AND IS THE POINT: §12 says conserve should
// "shift workload toward another healthy provider", and the shipped `mixed`
// profile already puts the reviewer capabilities on claude while the
// implementer ones are codex. Routing away from a constrained provider is the
// feature, so the fix is NOT to pin the shift to the evaluated provider.
//
// What was wrong is narrower and worse: the mode came from codex's capacity and
// the shift could hand the work to claude, whose capacity nobody had read. The
// decision then said "conserve, because codex is red" over a claude model. Now
// the landing provider's capacity is read in its own right and the move is
// refused when that provider is itself conserving — and the decision reports
// BOTH readings, so it cannot claim a reason it did not use.
func TestACrossProviderShiftIsCheckedAgainstTheProviderItLandsOn(t *testing.T) {
	zero := 0.0
	// `mixed` puts balanced on codex and deep_reviewer on claude, so this one
	// edit makes a codex-driven spend_down land the scout on claude.
	crossing, err := Load("test.yaml", []byte("mode_shifts:\n  spend_down:\n    scout: deep_reviewer\n"))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	codexSpendsDown := map[string]winSpec{"five_hour": {used: 12, resets: 75 * time.Minute}, "seven_day": {used: 11, resets: 96 * time.Hour}}
	claudeHealthy := map[string]winSpec{"five_hour": {used: 12, resets: 4 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}}
	claudeRed := map[string]winSpec{"five_hour": {used: 95, resets: 2 * time.Hour}, "seven_day": {used: 11, resets: 96 * time.Hour}}

	scout := Request{Role: "scout", ForecastDemandBeforeResetPct: &zero}

	t.Run("it applies when the landing provider can take the work, and says whose capacity said so", func(t *testing.T) {
		snap := snapshotOfProviders(t, "", map[string]map[string]winSpec{"codex": codexSpendsDown, "claude": claudeHealthy})
		d := Select(crossing, snap, nil, policyNow, scout)
		if d.Mode != ModeSpendDown {
			t.Fatalf("mode = %q, want spend_down — the rest proves nothing", d.Mode)
		}
		if d.Capability != "deep_reviewer" || d.Provider != "claude" || d.Model != "opus" {
			t.Fatalf("capability %q on %s %s, want deep_reviewer on claude opus — a shift that crosses providers is the feature, not the bug", d.Capability, d.Provider, d.Model)
		}
		if d.ModeProvider != "codex" {
			t.Errorf("ModeProvider = %q, want codex — the mode came from codex's expiring window, and a decision that cannot say so can claim a reason it did not use", d.ModeProvider)
		}
		if d.ShiftCapacity == nil {
			t.Fatal("the shift crossed to claude and no claude capacity was recorded — that is the defect: deciding on one provider's reading and spending another's")
		}
		if d.ShiftCapacity.Provider != "claude" || d.ShiftCapacity.Health != limits.HealthGreen {
			t.Errorf("ShiftCapacity = %+v, want a green claude reading", d.ShiftCapacity)
		}
		if d.ShiftMode != ModeNormal {
			t.Errorf("ShiftMode = %q, want normal (claude's own verdict)", d.ShiftMode)
		}
	})

	t.Run("it is REFUSED when the landing provider is itself conserving", func(t *testing.T) {
		snap := snapshotOfProviders(t, "", map[string]map[string]winSpec{"codex": codexSpendsDown, "claude": claudeRed})
		d := Select(crossing, snap, nil, policyNow, scout)
		if d.Mode != ModeSpendDown || d.ModeProvider != "codex" {
			t.Fatalf("mode %q from %q, want spend_down from codex", d.Mode, d.ModeProvider)
		}
		if d.Capability != "balanced" || d.Provider != "codex" || d.Model != "gpt-5.6-terra" {
			t.Fatalf("the scout was promoted onto a RED claude (%q on %s %s) because CODEX had capacity to spend — a shift onto a constrained provider because a different one was constrained is worse than not shifting",
				d.Capability, d.Provider, d.Model)
		}
		if d.ShiftCapacity == nil || d.ShiftCapacity.Health != limits.HealthRed || d.ShiftMode != ModeConserve {
			t.Errorf("the refused move did not record the reading it was refused on: %+v / %q", d.ShiftCapacity, d.ShiftMode)
		}
		if !strings.Contains(strings.Join(d.Reason, " "), "REFUSED") {
			t.Errorf("the refusal is silent: %v", d.Reason)
		}
	})

	t.Run("a disabled landing provider refuses the move without refusing the decision", func(t *testing.T) {
		m, err := Load("test.yaml", []byte("mode_shifts:\n  spend_down:\n    scout: deep_reviewer\nproviders:\n  claude:\n    enabled: false\n"))
		if err != nil {
			t.Fatalf("load: %v", err)
		}
		snap := snapshotOfProviders(t, "", map[string]map[string]winSpec{"codex": codexSpendsDown, "claude": claudeHealthy})
		d := Select(m, snap, nil, policyNow, scout)
		if !d.Eligible || d.Provider != "codex" || d.Capability != "balanced" {
			t.Fatalf("a disabled claude should cost the scout its promotion, not its assignment: %+v", d)
		}
		if !strings.Contains(strings.Join(d.Reason, " "), "enabled: false") {
			t.Errorf("the reason does not name the flag: %v", d.Reason)
		}
	})

	t.Run("a same-provider shift needs no second reading", func(t *testing.T) {
		// The shipped spend_down moves the scout balanced -> frontier, both on
		// codex under `mixed`. Nothing crosses, so nothing is re-read — and
		// ShiftCapacity staying nil is what says so.
		snap := snapshotOfProviders(t, "", map[string]map[string]winSpec{"codex": codexSpendsDown})
		d := Select(shipped(t), snap, nil, policyNow, Request{Role: "scout", Provider: "codex", ForecastDemandBeforeResetPct: &zero})
		if d.Capability != "frontier" || d.Provider != "codex" {
			t.Fatalf("got %q on %s", d.Capability, d.Provider)
		}
		if d.ShiftCapacity != nil {
			t.Errorf("a shift that stayed on codex recorded a second capacity: %+v", d.ShiftCapacity)
		}
		if d.ModeProvider != "codex" {
			t.Errorf("ModeProvider = %q, want codex", d.ModeProvider)
		}
	})
}

// ---------------------------------------------------------------------------
// §34 MANUAL MODE OVERRIDES
// ---------------------------------------------------------------------------

// TestManualModesFromTheFileReachTheDecision is §34's "auto | conserve | normal
// | spend_down, globally and optionally per provider".
//
// The per-provider half already had a case above. What this adds is the two
// arms that were never watched: `modes.global`, which is the spelling a user
// reaches for first, and the PRECEDENCE between the two — a global that
// silently outranked a per-provider entry would make "conserve Claude for the
// next few hours" unexpressible while codex stays automatic.
//
// It goes through Select, not DecideMode, because the mode is not the
// deliverable: the CAPABILITY the mode moves is. A manual mode that reached the
// verdict and not the `mode_shifts:` table would be a control that reports
// itself as applied and changes no dispatch.
func TestManualModesFromTheFileReachTheDecision(t *testing.T) {
	// A perfectly healthy, quiet codex — every automatic arm says NORMAL, so
	// anything else in the answers below came from the file and from nothing
	// else.
	healthy := snapshotOf(t, "codex", "acct", map[string]winSpec{
		"five_hour": {used: 12, resets: 4 * time.Hour},
		"seven_day": {used: 11, resets: 96 * time.Hour},
	})
	sel := func(t *testing.T, userYAML string) Decision {
		t.Helper()
		m, err := Load("test.yaml", []byte(userYAML))
		if err != nil {
			t.Fatalf("load: %v", err)
		}
		return Select(m, healthy, nil, policyNow, Request{Role: "scout", Provider: "codex"})
	}

	control := sel(t, "")
	if control.Mode != ModeNormal {
		t.Fatalf("the control case is %q, not normal — every case below would prove nothing", control.Mode)
	}
	if control.Capability != "balanced" {
		t.Fatalf("the control scout is %q, want balanced", control.Capability)
	}

	// modes.global alone.
	for _, tc := range []struct {
		yaml string
		want Mode
		cap  string
	}{
		{"modes:\n  global: conserve\n", ModeConserve, "cheap"},
		{"modes:\n  global: spend_down\n", ModeSpendDown, "frontier"},
		{"modes:\n  global: normal\n", ModeNormal, "balanced"},
	} {
		d := sel(t, tc.yaml)
		if d.Mode != tc.want || !d.ModeManual {
			t.Errorf("%q gave mode %q (manual=%v), want %q manual — modes.global is not reaching DecideMode",
				strings.TrimSpace(tc.yaml), d.Mode, d.ModeManual, tc.want)
		}
		if d.Capability != tc.cap {
			t.Errorf("%q left the scout on capability %q, want %q — the manual mode reached the verdict but not the mode_shifts table, which is a control that changes no dispatch\n  reasons: %s",
				strings.TrimSpace(tc.yaml), d.Capability, tc.cap, strings.Join(d.Reason, " | "))
		}
	}

	// PRECEDENCE: a per-provider entry wins over the global for that provider,
	// which is what makes "conserve Claude, leave codex automatic" sayable.
	d := sel(t, "modes:\n  global: conserve\n  providers:\n    codex: spend_down\n")
	if d.Mode != ModeSpendDown {
		t.Errorf("modes.providers.codex did not outrank modes.global (%q) — with the global winning, a per-provider control is unreachable", d.Mode)
	}
	// …and a provider the per-provider block does NOT name still follows the
	// global one.
	d = sel(t, "modes:\n  global: conserve\n  providers:\n    claude: normal\n")
	if d.Mode != ModeConserve {
		t.Errorf("codex, unnamed under modes.providers, did not follow modes.global (%q)", d.Mode)
	}

	// `auto` is not a mode: it defers, and must never appear on a decision.
	d = sel(t, "modes:\n  global: auto\n  providers:\n    codex: auto\n")
	if d.Mode != ModeNormal || d.ModeManual {
		t.Errorf("auto produced mode %q (manual=%v) — auto is the deferral, not a verdict", d.Mode, d.ModeManual)
	}

	// A mode the vocabulary does not have must not silently pin anything.
	d = sel(t, "modes:\n  global: consrve\n")
	if d.Mode != ModeNormal || d.ModeManual {
		t.Errorf("a misspelled mode was honoured as %q (manual=%v) — an unparseable override must fall through to the thresholds, not pin a mode nobody asked for", d.Mode, d.ModeManual)
	}
}
