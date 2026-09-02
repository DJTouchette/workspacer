package routing

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// ---------------------------------------------------------------------------
// EFFORT STEPPING
//
// One test per rule in effort.go, and each of them asserts the EFFORT the
// decision actually carries rather than only the sentence about it: a step that
// is explained and not applied is the same bug as one applied and not
// explained.
// ---------------------------------------------------------------------------

func stepSelect(t *testing.T, m *Matrix, snap limits.Snapshot, req Request) Decision {
	t.Helper()
	zero := 0.0
	if req.ForecastDemandBeforeResetPct == nil {
		req.ForecastDemandBeforeResetPct = &zero
	}
	return Select(m, snap, nil, nil, policyNow, req)
}

// uncappedShipped is the shipped matrix with its `default` ceiling raised to the
// top of the ladder.
//
// Several cases below need a decision that is ACTUALLY at frontier_max or
// frontier_plus, and the shipped default ceiling caps every directory at
// frontier — deliberately (see routing.default.yaml). Without this the ceiling
// would clamp the capability straight back down and the case would measure the
// ceiling instead of the effort step.
func uncappedShipped(t *testing.T, overlay string) *Matrix {
	t.Helper()
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier_plus, max_tool_scope: operator }\n"+overlay))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	return m
}

// spendDownWindows is a five-hour window resetting inside the shipped 90-minute
// spend-down horizon with 80% of it left, and a quiet week — the shape every
// spend-down arm accepts.
func spendDownWindows() map[string]winSpec {
	return map[string]winSpec{
		"five_hour": {used: 20, resets: 30 * time.Minute},
		"seven_day": {used: 11, resets: 96 * time.Hour},
	}
}

// TestConserveStepsTheEffortDownOneRung is the feature at its plainest: the same
// model, thinking less, on a provider whose allowance is nearly gone.
func TestConserveStepsTheEffortDownOneRung(t *testing.T) {
	m := shipped(t)
	// codex RED, and the request PINS codex — so the answer stays on the
	// constrained provider and the step is the only thing that can move.
	d := stepSelect(t, m, altSnapshot(t, altGreen, altRed), Request{
		Role: "implementer", Provider: "codex",
	})
	if !d.Eligible || d.Mode != ModeConserve {
		t.Fatalf("got %+v — this case needs a CONSERVE answer on codex to mean anything: %v", d, d.Reason)
	}
	if d.Capability != "frontier" || d.Model != "gpt-5.6-sol" {
		t.Fatalf("capability %q / model %q — the capability must NOT have moved; the effort step is the whole point", d.Capability, d.Model)
	}
	if d.Effort != "medium" {
		t.Fatalf("effort = %q, want `medium` — one rung down codex's own ladder from the declared `high`", d.Effort)
	}
	if d.EffortStep == nil {
		t.Fatal("the decision carries no effortStep record, so nothing downstream can say why the effort moved")
	}
	if d.EffortStep.From != "high" || d.EffortStep.To != "medium" || !d.EffortStep.Moved() {
		t.Errorf("effortStep = %+v, want high -> medium", d.EffortStep)
	}
	if !strings.Contains(strings.Join(d.Reason, " "), "steps down the codex ladder from `high` to `medium`") {
		t.Errorf("the step is not explained in the answer: %v", d.Reason)
	}
	// The final sentence has to agree with the field, or a reader gets two
	// different efforts out of one decision.
	if !strings.Contains(strings.Join(d.Reason, " "), "(effort medium)") {
		t.Errorf("the selection sentence still quotes the pre-step effort: %v", d.Reason)
	}
}

// TestSpendDownStepsTheEffortUpAndClampsAtTheLadderCeiling covers the other
// direction and the ceiling clamp in one case, because the second is only
// reachable through the first.
func TestSpendDownStepsTheEffortUpAndClampsAtTheLadderCeiling(t *testing.T) {
	m := uncappedShipped(t, "")
	snap := snapshotOf(t, "codex", "", spendDownWindows())

	// `supervisor` is frontier and has NO spend_down capability shift, so the
	// effort step is the only move available to it.
	up := stepSelect(t, m, snap, Request{Role: "supervisor", Provider: "codex", Profile: "codex_only"})
	if up.Mode != ModeSpendDown {
		t.Fatalf("this case needs SPEND_DOWN to mean anything: %s / %v", up.Mode, up.Reason)
	}
	if up.Effort != "xhigh" {
		t.Fatalf("effort = %q, want `xhigh` — one rung UP codex's ladder from `high`", up.Effort)
	}
	if up.EffortStep == nil || up.EffortStep.From != "high" || up.EffortStep.To != "xhigh" {
		t.Errorf("effortStep = %+v, want high -> xhigh", up.EffortStep)
	}

	// `judge` is frontier_plus, which codex_only already declares at `xhigh` —
	// the top of codex's ladder. The step is armed, runs out of ladder, and
	// SAYS SO rather than silently doing nothing.
	top := stepSelect(t, m, snap, Request{Role: "judge", Provider: "codex", Profile: "codex_only"})
	if top.Effort != "xhigh" {
		t.Fatalf("effort = %q — a step off the end of the ladder must clamp, not wrap or blank", top.Effort)
	}
	if top.EffortStep == nil || top.EffortStep.Moved() {
		t.Fatalf("effortStep = %+v, want an armed-but-unmoved record", top.EffortStep)
	}
	if !strings.Contains(top.EffortStep.Why, "clamped at codex's ceiling") {
		t.Errorf("the clamp is not explained: %q", top.EffortStep.Why)
	}
}

// TestAStepThroughTheLadderFloorClamps is the mirror, and it needs a row that
// already sits on the bottom rung — which the shipped file has none of, because
// nothing ships at `low`.
func TestAStepThroughTheLadderFloorClamps(t *testing.T) {
	m, err := Load("test.yaml", []byte(`
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-sol, effort: low }
`))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	d := stepSelect(t, m, altSnapshot(t, altGreen, altRed), Request{
		Role: "implementer", Provider: "codex", Profile: "codex_only",
	})
	if d.Mode != ModeConserve {
		t.Fatalf("this case needs CONSERVE: %s / %v", d.Mode, d.Reason)
	}
	if d.Effort != "low" {
		t.Fatalf("effort = %q, want `low` — the ladder has no rung below it", d.Effort)
	}
	if d.EffortStep == nil || !strings.Contains(d.EffortStep.Why, "clamped at codex's floor") {
		t.Errorf("effortStep = %+v, want the floor clamp said out loud", d.EffortStep)
	}
}

// TestAnAssignmentWithNoDeclaredEffortIsNotStepped is the rule that stops this
// feature inventing a level nobody wrote: a row with no `effort:` runs at the
// provider's own default, which this pure layer does not know.
func TestAnAssignmentWithNoDeclaredEffortIsNotStepped(t *testing.T) {
	m := uncappedShipped(t, "")
	// `judge` -> frontier_plus, which `mixed` spells `claude fable` with no
	// effort at all, and spend_down leaves the judge's capability alone.
	d := stepSelect(t, m, snapshotOf(t, "claude", "", spendDownWindows()), Request{
		Role: "judge", Provider: "claude",
	})
	if d.Mode != ModeSpendDown {
		t.Fatalf("this case needs SPEND_DOWN: %s / %v", d.Mode, d.Reason)
	}
	if d.Capability != "frontier_plus" || d.Model != "fable" {
		t.Fatalf("fixture drift: got %s / %s", d.Capability, d.Model)
	}
	if d.Effort != "" {
		t.Fatalf("effort = %q — a row that declares none must be left declaring none, not given a rung the file never named", d.Effort)
	}
	if d.EffortStep == nil || d.EffortStep.From != "" || d.EffortStep.To != "" {
		t.Fatalf("effortStep = %+v, want an armed record naming an empty from and to", d.EffortStep)
	}
	if !strings.Contains(d.EffortStep.Why, "declares no effort at all") {
		t.Errorf("the reason does not say WHY nothing moved: %q", d.EffortStep.Why)
	}
}

// TestTheMinEffortFloorHoldsAReviewCapability is knob 2: `min_effort` on the
// row, honoured against the mode's own step.
func TestTheMinEffortFloorHoldsAReviewCapability(t *testing.T) {
	m := shipped(t)
	// conserve moves the judge onto deep_reviewer, which `mixed` puts on claude
	// opus at `high` with `min_effort: high`. The step is armed, the capability
	// IS in the allow-list, and the floor is the only thing holding the effort.
	d := stepSelect(t, m, altSnapshot(t, altRed, altGreen), Request{
		Role: "judge", Provider: "claude",
	})
	if d.Mode != ModeConserve || d.Capability != "deep_reviewer" {
		t.Fatalf("this case needs a conserving judge on deep_reviewer: mode %s cap %s\n%v", d.Mode, d.Capability, d.Reason)
	}
	if d.Effort != "high" {
		t.Fatalf("effort = %q — `min_effort: high` on that row is what stops a conserving fleet from trimming its own reviewer", d.Effort)
	}
	if d.EffortStep == nil || !strings.Contains(d.EffortStep.Why, "min_effort floor") {
		t.Errorf("effortStep = %+v, want the floor named as the reason", d.EffortStep)
	}

	// AND THE FLOOR IS WHAT DID IT: the same decision against a matrix whose
	// floor is gone steps down. Without this, a `min_effort` that was parsed
	// and never read would pass the assertion above by accident, because the
	// capability might simply not have been steppable.
	nofloor := shipped(t)
	for pname, prof := range nofloor.Profiles {
		for capability, a := range prof {
			a.MinEffort = ""
			for i := range a.Alternatives {
				a.Alternatives[i].MinEffort = ""
			}
			prof[capability] = a
		}
		nofloor.Profiles[pname] = prof
	}
	loose := stepSelect(t, nofloor, altSnapshot(t, altRed, altGreen), Request{
		Role: "judge", Provider: "claude",
	})
	if loose.Effort != "medium" {
		t.Fatalf("with min_effort removed the same decision must step to `medium`; got %q — the floor above proved nothing", loose.Effort)
	}
}

// TestOnlyTheAllowListedCapabilitiesAreStepped is knob 3: the cheap tiers are
// left alone, and the frontier ones are not.
func TestOnlyTheAllowListedCapabilitiesAreStepped(t *testing.T) {
	m := shipped(t)
	snap := altSnapshot(t, altGreen, altRed)

	// conserve moves the scout from balanced down to cheap, which is NOT on the
	// list — `cheap` in codex_only is Luna with no effort anyway, so use
	// `validator`, whose conserve capability is the same and whose declared
	// effort is not. Either way the assertion is that nothing was trimmed.
	scout := stepSelect(t, m, snap, Request{Role: "scout", Provider: "codex", Profile: "codex_only"})
	if scout.Mode != ModeConserve {
		t.Fatalf("this case needs CONSERVE: %s / %v", scout.Mode, scout.Reason)
	}
	if scout.EffortStep == nil {
		t.Fatal("an armed step must still leave a record on a capability it declined to touch — otherwise nothing says why the trim did not happen")
	}
	if scout.EffortStep.Moved() {
		t.Errorf("capability %q was stepped despite not being in mode_shifts.conserve.effort_step_capabilities: %+v", scout.Capability, scout.EffortStep)
	}
	if !strings.Contains(scout.EffortStep.Why, "effort_step_capabilities") {
		t.Errorf("the reason does not name the allow-list: %q", scout.EffortStep.Why)
	}

	// The list is read from the FILE, not hard-coded: add the scout's
	// conserving capability to it and the same decision trims.
	widened, err := Load("test.yaml", []byte(`
profiles:
  codex_only:
    cheap: { provider: codex, model: gpt-5.6-luna, effort: high }
mode_shifts:
  conserve:
    effort_step_capabilities: [cheap, frontier, frontier_max, deep_reviewer, frontier_plus]
`))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	widenedScout := stepSelect(t, widened, snap, Request{Role: "scout", Provider: "codex", Profile: "codex_only"})
	if widenedScout.Capability != "cheap" || widenedScout.Effort != "medium" {
		t.Fatalf("got capability %q effort %q — the allow-list in the file is not what the router reads",
			widenedScout.Capability, widenedScout.Effort)
	}
}

// TestTheLowerPaceBandTrimsEffortAndLeavesTheCapabilityAlone is the ORDER rule.
//
// 60% of a five-hour window used at 50% elapsed is 1.20x: past
// block_spend_down_at_ratio and under conserve_at_ratio. Before this feature
// that band did exactly one thing — refuse a spend-down — and left an
// over-running window otherwise untouched. It now trims one notch, and it must
// NOT move the capability: that is the whole reason the step is armed before
// the shift rather than inside it.
func TestTheLowerPaceBandTrimsEffortAndLeavesTheCapabilityAlone(t *testing.T) {
	m := shipped(t)
	snap := snapshotOf(t, "claude", "", paceWindows(60, 150*time.Minute, 10, 84*time.Hour))
	d := stepSelect(t, m, snap, Request{Role: "implementer", Provider: "claude"})

	if d.Mode != ModeNormal {
		t.Fatalf("the lower band must NOT conserve — that is the higher one's job: %s\n%v", d.Mode, d.Reason)
	}
	if d.Capacity.Pace == nil || d.Capacity.Pace.State != limits.PaceAhead {
		t.Fatalf("fixture drift: pace = %+v, want the AHEAD band", d.Capacity.Pace)
	}
	if d.Capability != d.BaseCapability || d.Capability != "frontier" {
		t.Fatalf("capability moved to %q (base %q) — the lower band trims thinking time and nothing else", d.Capability, d.BaseCapability)
	}
	if d.Model != "opus" {
		t.Fatalf("model = %q — the same model must run the work", d.Model)
	}
	if d.Effort != "medium" {
		t.Fatalf("effort = %q, want `medium` — one notch off `high` because the window is running ahead of its curve", d.Effort)
	}
	if d.EffortStep == nil || !strings.Contains(d.EffortStep.Why, "block_spend_down_at_ratio") {
		t.Errorf("effortStep = %+v, want the band named", d.EffortStep)
	}

	// The band, not the mode: an ON-TRACK window at the same health is not
	// trimmed at all, which is what makes the case above a measurement of pace
	// rather than of something that fires everywhere.
	onTrack := stepSelect(t, m, snapshotOf(t, "claude", "", paceWindows(30, 150*time.Minute, 10, 84*time.Hour)),
		Request{Role: "implementer", Provider: "claude"})
	if onTrack.EffortStep != nil {
		t.Errorf("an on-track window was stepped: %+v", onTrack.EffortStep)
	}
	if onTrack.Effort != "high" {
		t.Errorf("effort = %q, want the declared `high` untouched", onTrack.Effort)
	}
}

// TestASpendDownThatAlreadyPromotedTheCapabilityDoesNotAlsoPromoteTheEffort is
// the cap: one promotion per decision.
func TestASpendDownThatAlreadyPromotedTheCapabilityDoesNotAlsoPromoteTheEffort(t *testing.T) {
	m := uncappedShipped(t, "")
	// implementer: frontier -> frontier_max under spend_down, which codex_only
	// already declares at `xhigh`. Without the cap the step would try to raise
	// it a second time off a tier that was itself just raised.
	d := stepSelect(t, m, snapshotOf(t, "codex", "", spendDownWindows()), Request{
		Role: "implementer", Provider: "codex", Profile: "codex_only",
	})
	if d.Mode != ModeSpendDown || d.Capability != "frontier_max" {
		t.Fatalf("this case needs a spend_down capability promotion: mode %s cap %s\n%v", d.Mode, d.Capability, d.Reason)
	}
	if d.Effort != "xhigh" {
		t.Errorf("effort = %q, want the promoted tier's own declared `xhigh`", d.Effort)
	}
	if d.EffortStep == nil || d.EffortStep.Moved() {
		t.Fatalf("effortStep = %+v, want an armed-but-unmoved record", d.EffortStep)
	}
	if !strings.Contains(d.EffortStep.Why, "one promotion per decision") {
		t.Errorf("the cap is not explained: %q", d.EffortStep.Why)
	}

	// AND THE CAP IS READ OFF THE RANKS, NOT OFF THE SHIFT FIRING. Under the
	// SHIPPED ceiling the same request is moved to frontier_max and clamped
	// straight back to frontier, so nothing was promoted and the step must
	// apply — the configuration this matrix actually ships with is the one
	// where a shift-based cap would have made the whole knob unreachable.
	clamped := stepSelect(t, shipped(t), snapshotOf(t, "codex", "", spendDownWindows()), Request{
		Role: "implementer", Provider: "codex", Profile: "codex_only",
	})
	if clamped.Capability != "frontier" {
		t.Fatalf("fixture drift: the shipped default ceiling should have clamped this back to frontier, got %q", clamped.Capability)
	}
	if clamped.Effort != "xhigh" {
		t.Errorf("effort = %q, want `xhigh` — the ceiling took the promotion back, so the step is the only promotion this decision got", clamped.Effort)
	}
}

// TestAFalloverTakesTheAlternativesEffortAndThenSteps is the composition rule:
// the walk chooses the row, and the step runs on THAT row's ladder and floor.
func TestAFalloverTakesTheAlternativesEffortAndThenSteps(t *testing.T) {
	m := shipped(t)
	// codex RED and UNCONSTRAINED: the mode comes from codex (the mixed
	// implementer's primary), the walk moves the answer to the claude
	// alternative, and the step then applies to claude's own row.
	d := stepSelect(t, m, altSnapshot(t, altGreen, altRed), Request{Role: "implementer"})
	if !d.Eligible || d.Provider != "claude" || d.Model != "opus" {
		t.Fatalf("got %+v, want the claude alternative — the rest of this test proves nothing otherwise", d)
	}
	if d.FellOverFrom == nil || d.FellOverFrom.Provider != "codex" {
		t.Fatalf("FellOverFrom = %+v, want the codex primary", d.FellOverFrom)
	}
	if d.Mode != ModeConserve {
		t.Fatalf("this case needs the SUBJECT's conserve to arm the step: %s", d.Mode)
	}
	if d.Effort != "medium" {
		t.Fatalf("effort = %q, want `medium` — one rung down CLAUDE's ladder from the alternative's own declared `high`", d.Effort)
	}
	if d.EffortStep == nil || d.EffortStep.From != "high" || d.EffortStep.To != "medium" {
		t.Fatalf("effortStep = %+v", d.EffortStep)
	}
	if !strings.Contains(d.EffortStep.Why, "claude opus steps down the claude ladder") {
		t.Errorf("the step names the wrong row — it must describe the alternative the answer landed on: %q", d.EffortStep.Why)
	}
}

// TestAProviderWithNoPublishedLadderIsNotStepped is rule 2's other half.
func TestAProviderWithNoPublishedLadderIsNotStepped(t *testing.T) {
	m, err := Load("test.yaml", []byte(`
profiles:
  codex_only:
    frontier: { provider: opencode, model: some-model, effort: high }
`))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	d := stepSelect(t, m, altSnapshot(t, altGreen, altRed), Request{
		Role: "implementer", Provider: "opencode", Profile: "codex_only",
	})
	if d.Effort != "high" {
		t.Errorf("effort = %q — a provider with no ladder here must keep what the file declares", d.Effort)
	}
	if d.EffortStep != nil && !strings.Contains(d.EffortStep.Why, "publishes no effort ladder") {
		t.Errorf("effortStep = %+v, want the missing ladder said out loud", d.EffortStep)
	}
}

// ---------------------------------------------------------------------------
// THE WIRING: every new knob reaches a DECISION
//
// Same shape as TestEveryPacingKnobReachesTheArithmetic, and for the same
// reason — this fleet's most common bug is a setting that is written and never
// read. The difference is the assertion: pacing's knobs are checked against the
// config struct they map onto, and these are checked against the ANSWER, which
// is the only place they exist.
// ---------------------------------------------------------------------------

type effortKnob struct {
	// fields are the ModeShift/Assignment field names this row covers.
	fields []string
	// mutate edits a freshly shipped matrix.
	mutate func(*Matrix)
	// want is what the decision must then say.
	want func(Decision) bool
}

// effortKnobs is one row per knob, each asserting a value that arrived.
func effortKnobs() []effortKnob {
	// Every row decides the same request: a conserving codex implementer, whose
	// capability (frontier) is on the shipped allow-list and whose row declares
	// `high`.
	setShift := func(m *Matrix, mode string, edit func(*ModeShift)) {
		s := m.ModeShifts[mode]
		edit(&s)
		m.ModeShifts[mode] = s
	}
	return []effortKnob{
		{
			fields: []string{"Roles"},
			mutate: func(m *Matrix) {
				setShift(m, "conserve", func(s *ModeShift) { s.Roles["implementer"] = "cheap" })
			},
			want: func(d Decision) bool { return d.Capability == "cheap" },
		},
		{
			fields: []string{"EffortStep"},
			mutate: func(m *Matrix) {
				setShift(m, "conserve", func(s *ModeShift) { s.EffortStep = -2 })
			},
			want: func(d Decision) bool { return d.Effort == "low" },
		},
		{
			fields: []string{"EffortStepCapabilities"},
			mutate: func(m *Matrix) {
				setShift(m, "conserve", func(s *ModeShift) { s.EffortStepCapabilities = []string{"cheap"} })
			},
			want: func(d Decision) bool { return d.Effort == "high" && d.EffortStep != nil && !d.EffortStep.Moved() },
		},
		{
			fields: []string{"MinEffort"},
			mutate: func(m *Matrix) {
				a := m.Profiles["codex_only"]["frontier"]
				a.MinEffort = "high"
				m.Profiles["codex_only"]["frontier"] = a
			},
			want: func(d Decision) bool { return d.Effort == "high" && strings.Contains(d.EffortStep.Why, "min_effort") },
		},
	}
}

func TestEveryEffortKnobReachesTheDecision(t *testing.T) {
	for _, knob := range effortKnobs() {
		name := strings.Join(knob.fields, ".")
		t.Run(name, func(t *testing.T) {
			m := shipped(t)
			knob.mutate(m)
			d := stepSelect(t, m, altSnapshot(t, altGreen, altRed), Request{
				Role: "implementer", Provider: "codex", Profile: "codex_only",
			})
			if d.Mode != ModeConserve {
				t.Fatalf("fixture drift: this table needs a CONSERVE answer, got %s\n%v", d.Mode, d.Reason)
			}
			if !knob.want(d) {
				t.Errorf("the %s knob is parsed and never reaches a decision — a setting that is written and never read looks exactly like a working one. effort=%q capability=%q step=%+v",
					name, d.Effort, d.Capability, d.EffortStep)
			}
		})
	}
}

// TestTheEffortKnobTableIsComplete is the guard on the guard: a field added to
// ModeShift with no row above would be both untested and unread.
func TestTheEffortKnobTableIsComplete(t *testing.T) {
	covered := map[string]bool{}
	for _, k := range effortKnobs() {
		for _, f := range k.fields {
			covered[f] = true
		}
	}
	seen := 0
	ty := reflect.TypeOf(ModeShift{})
	for i := 0; i < ty.NumField(); i++ {
		seen++
		if f := ty.Field(i); !covered[f.Name] {
			t.Errorf("mode_shifts carries field %q that no effortKnobs() row mutates — add a row, and check the value actually reaches a Decision", f.Name)
		}
	}
	if seen < 3 {
		t.Fatalf("walked only %d mode_shifts fields — the struct was renamed and this guard is guarding nothing", seen)
	}
	if !covered["MinEffort"] {
		t.Error("nothing covers the per-assignment min_effort floor")
	}
	if _, ok := reflect.TypeOf(Assignment{}).FieldByName("MinEffort"); !ok {
		t.Error("Assignment lost its MinEffort field and the floor rows above are testing nothing")
	}
}

// ---------------------------------------------------------------------------
// LOAD-TIME VALIDATION
// ---------------------------------------------------------------------------

func TestEffortSteppingValidationReportsUnusableKnobs(t *testing.T) {
	for _, tc := range []struct {
		name string
		yaml string
		at   string
		want string
	}{
		{
			"a conserve step that steps UP",
			"mode_shifts:\n  conserve:\n    effort_step: 1\n",
			"mode_shifts.conserve", "POSITIVE step under conserve",
		},
		{
			"a spend_down step that steps DOWN",
			"mode_shifts:\n  spend_down:\n    effort_step: -1\n",
			"mode_shifts.spend_down", "NEGATIVE step under spend_down",
		},
		{
			"an allow-list naming a capability that does not exist",
			"mode_shifts:\n  conserve:\n    effort_step_capabilities: [frontier, forntier_max]\n",
			"mode_shifts.conserve.effort_step_capabilities", "not in the `capabilities:` list",
		},
		{
			"a floor that is not on the provider's ladder",
			"profiles:\n  mixed:\n    frontier:\n      min_effort: enormous\n",
			"profiles.mixed.frontier", "not on codex's effort ladder",
		},
		{
			"a floor above the effort the row declares",
			"profiles:\n  mixed:\n    frontier:\n      min_effort: xhigh\n",
			"profiles.mixed.frontier", "is ABOVE the effort",
		},
		{
			"a floor on an ALTERNATIVE is validated too",
			"profiles:\n  mixed:\n    frontier:\n      alternatives:\n        - { provider: claude, model: opus, effort: high, min_effort: nonsense }\n",
			"profiles.mixed.frontier.alternatives[0]", "not on claude's effort ladder",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			m, err := Load("test.yaml", []byte(tc.yaml))
			if err != nil {
				t.Fatalf("load: %v", err)
			}
			found := false
			for _, iss := range m.Issues {
				if iss.Where == tc.at && strings.Contains(iss.Detail, tc.want) {
					found = true
				}
			}
			if !found {
				t.Errorf("no issue at %q mentioning %q; got %v", tc.at, tc.want, m.Issues)
			}
		})
	}
}

// TestTheShippedEffortSteppingDefaultsAreTheDocumentedOnes pins what a fresh
// install gets, and that the shipped file does not report itself as broken.
func TestTheShippedEffortSteppingDefaultsAreTheDocumentedOnes(t *testing.T) {
	m := shipped(t)
	conserve, ok := m.EffortStepFor(ModeConserve)
	if !ok {
		t.Fatal("the shipped matrix has no conserve block at all")
	}
	if conserve.EffortStep != -1 {
		t.Errorf("conserve ships at -1 (one rung down), got %+d", conserve.EffortStep)
	}
	spend, _ := m.EffortStepFor(ModeSpendDown)
	if spend.EffortStep != 1 {
		t.Errorf("spend_down ships at +1, got %+d", spend.EffortStep)
	}
	want := []string{"frontier", "frontier_max", "deep_reviewer", "frontier_plus"}
	for _, block := range []ModeShift{conserve, spend} {
		if !reflect.DeepEqual(block.EffortStepCapabilities, want) {
			t.Errorf("the shipped allow-list moved: %v, want %v — cheap and balanced are left alone on purpose", block.EffortStepCapabilities, want)
		}
	}
	// The review floors, which are the reason min_effort exists.
	for _, profile := range []string{"mixed", "anthropic_only"} {
		for _, capability := range []string{"reviewer", "deep_reviewer", "frontier_plus"} {
			a, err := m.ResolveCapability(profile, capability)
			if err != nil {
				t.Fatalf("%s.%s: %v", profile, capability, err)
			}
			if a.MinEffort != "high" {
				t.Errorf("profiles.%s.%s has min_effort %q, want `high` — a review that is trimmed is not a review", profile, capability, a.MinEffort)
			}
			for i, alt := range a.Alternatives {
				if alt.MinEffort != "high" {
					t.Errorf("profiles.%s.%s.alternatives[%d] has min_effort %q — a floor on the primary alone stops binding the moment a fallover moves the answer",
						profile, capability, i, alt.MinEffort)
				}
			}
		}
	}
	for _, issue := range m.Issues {
		if strings.HasPrefix(issue.Where, "mode_shifts") || strings.Contains(issue.Detail, "min_effort") {
			t.Errorf("the shipped matrix reports its own effort block as an issue: %s", issue)
		}
	}
}

// TestTheRolesTableStillParsesBesideTheReservedKeys is the compatibility check
// for the inline map: a routing.yaml written before effort stepping existed
// still means exactly what it meant.
func TestTheRolesTableStillParsesBesideTheReservedKeys(t *testing.T) {
	m, err := Load("test.yaml", []byte(`
mode_shifts:
  conserve:
    implementer: balanced
`))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	got, ok := m.ShiftFor("conserve", "implementer")
	if !ok || got != "balanced" {
		t.Fatalf("ShiftFor = %q/%v, want the user's own role entry", got, ok)
	}
	if s := m.ModeShifts["conserve"]; s.EffortStep != -1 {
		t.Errorf("the user's role entry ate the shipped effort_step: %+v", s)
	}
	if _, ok := m.ModeShifts["conserve"].Roles["effort_step"]; ok {
		t.Error("`effort_step` was parsed as a ROLE — the reserved keys must not land in the inline map, or every mode shift would try to resolve a capability called -1")
	}
	if _, ok := m.ShiftFor("conserve", "effort_step_capabilities"); ok {
		t.Error("`effort_step_capabilities` is reachable as a role")
	}
}

// TestTheUsersOwnRoutingFileStillLoadsCleanly reads the developer's real
// routing.yaml, READ-ONLY, and requires this release's new knobs not to have
// broken it.
//
// It is skipped when there is no such file, which is every CI machine — the
// value is on the machine where the file exists and has been hand-edited, which
// is precisely where a schema change bites first.
func TestTheUsersOwnRoutingFileStillLoadsCleanly(t *testing.T) {
	dir, err := os.UserConfigDir()
	if err != nil || dir == "" {
		t.Skip("no user config dir on this host")
	}
	path := filepath.Join(dir, "workspacer-hub", "routing.yaml")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Skipf("no routing.yaml at %s", path)
	}
	m, err := Load(path, raw)
	if err != nil {
		t.Fatalf("the routing.yaml on this machine no longer parses: %v", err)
	}
	for _, iss := range m.Issues {
		t.Errorf("%s reports an issue against this release's schema: %s", path, iss)
	}
	if _, ok := m.EffortStepFor(ModeConserve); !ok {
		t.Error("the merged matrix carries no conserve block — the deep merge over the shipped defaults stopped working")
	}
	// And it still answers. A file that loads and cannot route is not loading.
	d := stepSelect(t, m, altSnapshot(t, altGreen, altGreen), Request{Role: "implementer"})
	if !d.Eligible {
		t.Errorf("this machine's own matrix cannot route an implementer: %v", d.Reason)
	}
}

// TestEffortSteppingOffIsTheAnswerFromBeforeItExisted is the compatibility
// guard, in the same form as
// TestAMatrixWithNoAlternativesDecidesByteForByteAsBefore: with
// `effort_step: 0` the whole feature must be inert.
//
// It runs the comparison TWICE, because there are two honest versions of the
// claim and they differ by one field:
//
//	no floors written   a routing.yaml that carries no `min_effort:` anywhere
//	                    (every file written before this release) decides BYTE
//	                    FOR BYTE as it did, with no allowance made.
//	floors written      the shipped file now writes `min_effort: high` on its
//	                    review rows, and a decision that FELL OVER echoes the
//	                    primary it passed over — so that one row reports the
//	                    floor the file states. It is the file's own content
//	                    being read back on an additive field, it changes no
//	                    routing outcome and no reason line, and it is the one
//	                    difference this test allows for, by name.
func TestEffortSteppingOffIsTheAnswerFromBeforeItExisted(t *testing.T) {
	zero := 0.0
	stripFloors := func(m *Matrix) *Matrix {
		for pname, prof := range m.Profiles {
			for capability, a := range prof {
				a.MinEffort = ""
				for i := range a.Alternatives {
					a.Alternatives[i].MinEffort = ""
				}
				prof[capability] = a
			}
			m.Profiles[pname] = prof
		}
		return m
	}
	stripSteps := func(m *Matrix, alsoTheList bool) *Matrix {
		for _, mode := range []string{"conserve", "spend_down"} {
			s := m.ModeShifts[mode]
			s.EffortStep = 0
			if alsoTheList {
				s.EffortStepCapabilities = nil
			}
			m.ModeShifts[mode] = s
		}
		return m
	}

	// `before` is the matrix as it would have been written before this feature
	// existed: no step, no allow-list, no floors.
	before := stripFloors(stripSteps(shipped(t), true))
	// `offNoFloors` keeps the allow-list — a knob that must do nothing on its
	// own — and writes no floors.
	offNoFloors := stripFloors(stripSteps(shipped(t), false))
	// `offWithFloors` is the shipped file with only the step switched off.
	offWithFloors := stripSteps(shipped(t), false)

	// The one field the shipped floors legitimately add to an answer: the
	// echoed primary a fallover passed over.
	dropFloorEcho := func(doc string) string {
		return strings.ReplaceAll(doc, `,"minEffort":"high"`, "")
	}

	for _, snap := range []limits.Snapshot{
		altSnapshot(t, altGreen, altGreen),
		altSnapshot(t, altRed, altGreen),
		altSnapshot(t, altGreen, altRed),
		snapshotOf(t, "claude", "", spendDownWindows()),
		snapshotOf(t, "claude", "", paceWindows(60, 150*time.Minute, 10, 84*time.Hour)),
	} {
		for _, req := range []Request{
			{Role: "scout", ForecastDemandBeforeResetPct: &zero},
			{Role: "reviewer", ForecastDemandBeforeResetPct: &zero},
			{Role: "implementer", ForecastDemandBeforeResetPct: &zero},
			{Role: "implementer", Provider: "claude", ForecastDemandBeforeResetPct: &zero},
			{Role: "judge", Provider: "codex", ForecastDemandBeforeResetPct: &zero},
			{Role: "reviewer", PreviousProvider: "codex", RequireIndependentFamily: true, ForecastDemandBeforeResetPct: &zero},
		} {
			want, err := json.Marshal(Select(before, snap, nil, nil, policyNow, req))
			if err != nil {
				t.Fatal(err)
			}
			plain, err := json.Marshal(Select(offNoFloors, snap, nil, nil, policyNow, req))
			if err != nil {
				t.Fatal(err)
			}
			if string(plain) != string(want) {
				t.Errorf("role %s: with effort_step 0 and no floors written, the answer is not the pre-stepping one\n  got: %s\n want: %s", req.Role, plain, want)
			}
			floors, err := json.Marshal(Select(offWithFloors, snap, nil, nil, policyNow, req))
			if err != nil {
				t.Fatal(err)
			}
			if dropFloorEcho(string(floors)) != string(want) {
				t.Errorf("role %s: the shipped min_effort floors changed an answer no step was armed for\n  got: %s\n want: %s", req.Role, floors, want)
			}
			for _, doc := range []string{string(plain), string(floors)} {
				if strings.Contains(doc, "effortStep") {
					t.Errorf("role %s: a decision carries an effortStep record with stepping switched off: %s", req.Role, doc)
				}
				if strings.Contains(doc, "effort_step") {
					t.Errorf("role %s: a reason line talks about effort stepping with it switched off: %s", req.Role, doc)
				}
			}
		}
	}
}

// ---------------------------------------------------------------------------
// A PACE-ARMED STEP IS PER-PROVIDER
//
// The lower band's argument is "THIS provider is spending a little fast". It is
// an argument about the provider it was read from, so it has to be re-asked of
// the provider the answer finally lands on. Both cases below are the SAME
// matrix, the same role and the same unavailable codex; only the landing
// provider's own window progress differs, which is what makes the pair a
// measurement of the band rather than of anything else.
// ---------------------------------------------------------------------------

// paceFallover is the shipped `mixed` implementer with codex unavailable, so the
// walk always lands on the claude alternative — and claude's own pace is the
// variable.
func paceFallover(t *testing.T, claude map[string]winSpec) Decision {
	t.Helper()
	zero := 0.0
	snap := snapshotOfProviders(t, "acct", map[string]map[string]winSpec{
		"claude": claude,
		// 60% of a five-hour window at 50% elapsed: 1.15x, past
		// block_spend_down_at_ratio and under conserve_at_ratio.
		"codex": paceWindows(60, 150*time.Minute, 11, 96*time.Hour),
	})
	return Select(shipped(t), snap, nil,
		ProviderAvailability{"codex": {Available: false, Reason: "codex answered the model catalog with no launchable model"}},
		policyNow, Request{Role: "implementer", ForecastDemandBeforeResetPct: &zero})
}

// TestAPaceArmedStepDoesNotCarryAcrossAFallover is SHOULD-FIX 1 on the exact
// shape the reviewer found on shipped defaults: an ahead-of-curve codex that is
// also unavailable used to trim a healthy, on-track claude one notch and quote
// CODEX's ratio as the reason.
func TestAPaceArmedStepDoesNotCarryAcrossAFallover(t *testing.T) {
	// claude: 5% of the five-hour window used, comfortably inside the curve.
	d := paceFallover(t, paceWindows(5, 150*time.Minute, 5, 96*time.Hour))

	if !d.Eligible || d.Provider != "claude" || d.Model != "opus" {
		t.Fatalf("got %+v, want the claude alternative — the rest of this case proves nothing otherwise: %v", d, d.Reason)
	}
	if d.Mode != ModeNormal {
		t.Fatalf("mode = %s, want NORMAL — this case is about the PACE band, not about a mode: %v", d.Mode, d.Reason)
	}
	if d.Capacity.Pace == nil || d.Capacity.Pace.State != limits.PaceAhead {
		t.Fatalf("fixture drift: the codex subject's pace = %+v, want the AHEAD band that arms the step", d.Capacity.Pace)
	}
	if land := d.EffectiveCapacity(); land.Pace == nil || land.Pace.State != limits.PaceOnTrack {
		t.Fatalf("fixture drift: the landing claude's pace = %+v, want ON TRACK", land.Pace)
	}

	if d.Effort != "high" {
		t.Fatalf("effort = %q, want the declared `high` — codex's window progress is not an argument about claude's", d.Effort)
	}
	if d.EffortStep == nil {
		t.Fatal("a step that was armed and then refused must still be published: an operator who wrote effort_step: -1 and sees no trim has to be able to learn why")
	}
	if d.EffortStep.Moved() {
		t.Fatalf("effortStep = %+v, want an armed-but-unmoved record", d.EffortStep)
	}
	if !strings.Contains(d.EffortStep.Why, "did not carry") {
		t.Errorf("the answer does not say the pace step failed to carry across the fallover: %q", d.EffortStep.Why)
	}
	if !strings.Contains(d.EffortStep.Why, "claude's own pace is on_track") {
		t.Errorf("the refusal does not name the LANDING provider's own pace, which is the fact it turns on: %q", d.EffortStep.Why)
	}
	// And it must not read as though claude were the one running ahead.
	if strings.Contains(d.EffortStep.Why, "claude is running at") {
		t.Errorf("the reason attributes codex's overspend to claude: %q", d.EffortStep.Why)
	}
}

// TestAPaceArmedStepAppliesWhenTheLandingProviderIsItselfBehindPace is the
// mirror, and it is what stops the fix above from being "the pace band never
// steps a fallover".
func TestAPaceArmedStepAppliesWhenTheLandingProviderIsItselfBehindPace(t *testing.T) {
	// claude is running ahead too, on its own reading.
	d := paceFallover(t, paceWindows(60, 150*time.Minute, 11, 96*time.Hour))

	if !d.Eligible || d.Provider != "claude" || d.Model != "opus" {
		t.Fatalf("got %+v, want the claude alternative: %v", d, d.Reason)
	}
	if d.Mode != ModeNormal {
		t.Fatalf("mode = %s, want NORMAL: %v", d.Mode, d.Reason)
	}
	if land := d.EffectiveCapacity(); land.Pace == nil || land.Pace.State != limits.PaceAhead {
		t.Fatalf("fixture drift: the landing claude's pace = %+v, want the AHEAD band", land.Pace)
	}
	if d.Effort != "medium" {
		t.Fatalf("effort = %q, want `medium` — claude is itself past the lower band, so the trim is its own", d.Effort)
	}
	if d.EffortStep == nil || d.EffortStep.From != "high" || d.EffortStep.To != "medium" {
		t.Fatalf("effortStep = %+v, want high -> medium", d.EffortStep)
	}
	if !strings.Contains(d.EffortStep.Why, "claude is running at 1.15x") {
		t.Errorf("the reason must name the provider whose pace justified the step, and its own ratio: %q", d.EffortStep.Why)
	}
	if strings.Contains(d.EffortStep.Why, "codex is running at") {
		t.Errorf("the reason quotes the subject's ratio for a step taken on the landing provider: %q", d.EffortStep.Why)
	}
}

// TestAFloorAboveTheRowsOwnEffortDisablesStepping is SHOULD-FIX 3, and the rule
// is stated once here, in routing.default.yaml and in docs/limit-aware-routing.md:
// `min_effort` above the declared `effort` is a load-time validation issue that
// SKIPS stepping for that row. A floor holds an effort down; it never raises one.
func TestAFloorAboveTheRowsOwnEffortDisablesStepping(t *testing.T) {
	m, err := Load("test.yaml", []byte(`
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-sol, effort: low, min_effort: high }
`))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if !hasIssueAt(m.Issues, "profiles.codex_only.frontier") {
		t.Fatalf("the loader does not report the contradiction at all: %v", m.Issues)
	}

	// Both doors into that row, because the fallover walk's Issue filter guards
	// neither of them: a PINNED request never walks, and codex_only declares no
	// `alternatives:` anywhere, so the walk returns before it can filter.
	for _, tc := range []struct {
		name string
		req  Request
	}{
		{"a pinned provider", Request{Role: "implementer", Provider: "codex", Profile: "codex_only"}},
		{"a capability with no alternatives", Request{Role: "implementer", Profile: "codex_only"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			d := stepSelect(t, m, altSnapshot(t, altGreen, altRed), tc.req)
			if d.Mode != ModeConserve {
				t.Fatalf("this case needs CONSERVE to arm a -1 step: %s / %v", d.Mode, d.Reason)
			}
			if d.Capability != "frontier" {
				t.Fatalf("capability = %q, want the allow-listed frontier", d.Capability)
			}
			if d.Effort != "low" {
				t.Fatalf("effort = %q, want the declared `low` untouched — a min_effort floor must never raise an effort", d.Effort)
			}
			if d.EffortStep == nil || d.EffortStep.Moved() {
				t.Fatalf("effortStep = %+v, want an armed-but-unmoved record", d.EffortStep)
			}
			if !strings.Contains(d.EffortStep.Why, "stepping is SKIPPED") {
				t.Errorf("the answer does not say stepping was skipped for this row: %q", d.EffortStep.Why)
			}
			if !strings.Contains(d.EffortStep.Why, "is ABOVE the `low`") {
				t.Errorf("the answer does not name the contradiction it skipped on: %q", d.EffortStep.Why)
			}
		})
	}

	// The floor is what did it: the same row with a legal floor steps normally,
	// so the case above measures the contradiction rather than an unsteppable
	// row.
	legal, err := Load("test.yaml", []byte(`
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-sol, effort: high, min_effort: low }
`))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	ok := stepSelect(t, legal, altSnapshot(t, altGreen, altRed), Request{
		Role: "implementer", Provider: "codex", Profile: "codex_only",
	})
	if ok.Effort != "medium" {
		t.Fatalf("effort = %q with a floor BELOW the declared effort; the skip above proved nothing", ok.Effort)
	}
}

// TestAPositiveConserveStepCannotArmTheLowerPaceBand is SHOULD-FIX 4: the
// direction guard in armEffortStep, asserted as an ANSWER rather than as a
// branch.
//
// The band means "spending a little fast". A matrix whose conserve block steps
// UP would otherwise answer that by spending faster still, so the guard refuses
// to arm at all — and refusing to arm has to be byte-identical to there being no
// step configured, reasons and effortStep field included.
func TestAPositiveConserveStepCannotArmTheLowerPaceBand(t *testing.T) {
	m, err := Load("test.yaml", []byte("mode_shifts:\n  conserve:\n    effort_step: 1\n"))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if m.ModeShifts["conserve"].EffortStep != 1 {
		t.Fatalf("fixture drift: conserve.effort_step = %d, want the POSITIVE step this case is about", m.ModeShifts["conserve"].EffortStep)
	}
	// The same lower-band window TestTheLowerPaceBandTrimsEffortAndLeavesTheCapabilityAlone
	// uses, so a step that armed here would be visible as an effort change.
	snap := snapshotOf(t, "claude", "", paceWindows(60, 150*time.Minute, 10, 84*time.Hour))
	req := Request{Role: "implementer", Provider: "claude"}

	d := stepSelect(t, m, snap, req)
	if d.Mode != ModeNormal || d.Capacity.Pace == nil || d.Capacity.Pace.State != limits.PaceAhead {
		t.Fatalf("this case needs a NORMAL decision in the AHEAD band: mode %s pace %+v", d.Mode, d.Capacity.Pace)
	}
	if d.Effort != "high" {
		t.Fatalf("effort = %q — a provider already ahead of its curve must not be asked to think LONGER", d.Effort)
	}
	if d.EffortStep != nil {
		t.Fatalf("effortStep = %+v, want nothing armed at all", d.EffortStep)
	}

	// BYTE-IDENTICAL to the same matrix with no step configured. The comparison
	// is against an in-memory edit of THIS matrix rather than a second Load, so
	// the load-time Issue a positive conserve step earns (and its count on
	// Decision.Matrix) is held constant and the only difference is the knob.
	shift := m.ModeShifts["conserve"]
	shift.EffortStep = 0
	m.ModeShifts["conserve"] = shift
	unarmed := stepSelect(t, m, snap, req)

	got, err := json.Marshal(d)
	if err != nil {
		t.Fatal(err)
	}
	want, err := json.Marshal(unarmed)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(want) {
		t.Errorf("a POSITIVE conserve step changed the lower band's answer\n got: %s\nwant: %s", got, want)
	}
}
