package routing

// EFFORT STEPPING: the gentler move, one notch at a time.
//
// A routing mode has had exactly one lever since this feature existed — move
// the ROLE onto a different CAPABILITY — and that lever is a blunt one. It
// changes which model runs the work, so conserving means a scout stops being a
// Sol scout and becomes a Luna one, and spending down means an implementer is
// handed a bigger model than the work asked for. Between "the same model" and
// "a different model" there is a move nothing could make: THE SAME MODEL,
// THINKING LESS. That is what a step on the provider's own reasoning-effort
// ladder is, and this file is all of it.
//
// THREE RULES DECIDE WHETHER A STEP HAPPENS AT ALL, and each of them exists
// because the obvious implementation gets it wrong:
//
//  1. AN ASSIGNMENT WITH NO DECLARED EFFORT IS NOT STEPPED. `frontier_plus:
//     {provider: claude, model: fable}` runs at whatever the provider's own
//     default is, and this layer does not know what that is — claudemon reports
//     a defaultEffort per model, but Select is pure and reads no catalog. There
//     is no rung to count notches from, so counting one would be inventing a
//     level the operator never wrote. The answer says so instead.
//
//  2. THE LADDERS ARE NOT PORTABLE. claude runs low..max, codex stops at xhigh,
//     copilot starts below both at `none`. A step is therefore a NOTCH COUNT
//     rather than a level name: `-1` means the same thing on every provider,
//     and `medium` does not. A provider with no ladder here (opencode, pi — BYO
//     key, no published ladder) is not stepped, for the same reason as rule 1.
//
//  3. NOT EVERY CAPABILITY IS WORTH STEPPING. Trimming thinking time off a
//     frontier tier saves real allowance; trimming it off a cheap one saves
//     very little and costs a noticeably worse answer. So the mode names the
//     capabilities its step applies to, and the shipped list is the four tiers
//     where reasoning is the expensive part.
//
// AND ONE RULE DECIDES HOW FAR: the step clamps at the ladder's own ends, and
// at the assignment's `min_effort` floor when it has one. Both clamps are
// REPORTED rather than silent — "we wanted to step down and could not" is a
// different fact from "we did not try", and an operator tuning a matrix needs
// to be able to tell them apart.

import (
	"fmt"
	"strings"
)

// effortLadders is each provider's own reasoning-effort ladder, weakest rung
// first.
//
// It is a table in Go rather than a block in routing.yaml because it is a
// VOCABULARY, not a policy: these are the levels the CLIs accept, and a fleet
// operator inventing a sixth claude level in a config file would produce a
// spawn the CLI refuses. Every entry is taken from the adapter that builds the
// argv, so there is one source per provider and it is the one that runs:
//
//	claude    apps/desktop/src/renderer/src/lib/providerCaps.ts's
//	          CLAUDE_EFFORT_LEVELS — the `--effort` launch flag's own list,
//	          deliberately NOT the `/effort` slash command's wider vocabulary.
//	codex     services/claudemon/src/providers/codex.rs, which reports the
//	          ladder per model from `model/list`; every model it serves offers
//	          these four.
//	copilot   services/claudemon/src/providers/copilot.rs's EFFORT_LEVELS,
//	          printed by `copilot --help` on v1.0.81.
//
// opencode and pi are ABSENT on purpose: they are BYO-key harnesses with no
// published effort ladder, and an absent ladder means "not stepped, and said
// out loud" rather than "stepped against a guess". THE LADDER IS NOT A CHECK
// ON THE MATRIX — ValidateAgainstCatalog still validates a declared effort
// against what the installed CLI actually reports, which is the live answer;
// this table only says which rung is next.
var effortLadders = map[string][]string{
	"claude":  {"low", "medium", "high", "xhigh", "max"},
	"codex":   {"low", "medium", "high", "xhigh"},
	"copilot": {"none", "minimal", "low", "medium", "high", "xhigh", "max"},
}

// EffortLadder is a provider's ladder, weakest rung first. ok is false for a
// provider with no published ladder.
func EffortLadder(provider string) ([]string, bool) {
	ladder, ok := effortLadders[normalizeProvider(provider)]
	return ladder, ok
}

// effortRung is the position of an effort level on a provider's ladder.
func effortRung(provider, effort string) (int, bool) {
	ladder, ok := EffortLadder(provider)
	if !ok {
		return 0, false
	}
	want := strings.ToLower(strings.TrimSpace(effort))
	for i, level := range ladder {
		if level == want {
			return i, true
		}
	}
	return 0, false
}

// EffortStep is the additive record of what a mode's effort step did, or did
// not do, to a decision.
//
// It is a POINTER on the Decision with `omitempty`, and it is set only when a
// step was actually ARMED — a mode with `effort_step: 0`, or a decision no band
// armed, carries no such field and no reason sentence, which is what makes the
// whole feature reproduce the pre-stepping answer byte for byte.
type EffortStep struct {
	// From is the effort the landing assignment declares — empty when it
	// declares none, which is itself a reason not to step.
	From string `json:"from,omitempty"`
	// To is the effort the decision actually carries. Equal to From when the
	// step was armed and then clamped, floored or refused; the pair is what
	// says "we looked and did not move" rather than "we never looked".
	To string `json:"to,omitempty"`
	// Why is the one sentence explaining both, and it is the same sentence the
	// decision's reason list carries.
	Why string `json:"why"`
}

// Moved reports whether the step actually changed the effort.
func (e EffortStep) Moved() bool { return e.From != e.To }

// effortIntent is the ARMING decision, taken in Select immediately after the
// mode and BEFORE the capability shift, and applied to whatever assignment the
// answer eventually lands on.
//
// The two halves are deliberately separated in time. What arms a step is a fact
// about the SUBJECT provider's capacity — the same reading the mode came from —
// and what a step applies to is the assignment the walk and the ceiling
// finally chose, which may be a different provider entirely with a different
// ladder. Deciding both at once would either judge the wrong capacity or step
// the wrong ladder.
type effortIntent struct {
	armed   bool
	notches int
	// mode names the mode_shifts block the notch count and the allow-list came
	// from, which is not always the decision's own mode: the pace band below
	// arms a CONSERVE-shaped trim on a decision whose mode is still normal.
	mode Mode
	// band is the sentence fragment naming what armed the step.
	band string
	// shift is the mode_shifts block the count came from, carried whole so the
	// allow-list is read from the same block as the count rather than looked up
	// again later against a matrix the application site would have to be handed.
	shift ModeShift
}

// EffortStepFor is the mode's effort block: the notch count and the
// capabilities it applies to.
func (m *Matrix) EffortStepFor(mode Mode) (ModeShift, bool) {
	if m == nil {
		return ModeShift{}, false
	}
	s, ok := m.ModeShifts[strings.ToLower(strings.TrimSpace(string(mode)))]
	return s, ok
}

// armEffortStep is step 6b of Select: does this decision want an effort step,
// and of how many notches?
//
// THE ORDER IS THE POINT. It runs after the mode is decided and before the
// capability shift, because the two moves answer to different bands of the same
// evidence and the gentler one has to be reachable without the harsher one:
//
//	CONSERVE / SPEND_DOWN   the mode's own `effort_step`, alongside the
//	                        capability shift the mode already performs.
//	NORMAL, pace AHEAD      the LOWER overspend band — consumption is past
//	                        `block_spend_down_at_ratio` but has not reached
//	                        `conserve_at_ratio`. Today that band blocks a
//	                        spend-down and does nothing else. It now also trims
//	                        one notch of thinking time, using CONSERVE's own
//	                        step, and it still does not move the capability:
//	                        being slightly ahead of the curve is a reason to
//	                        spend a little less, not a reason to change which
//	                        model does the work.
//
// A conserve step configured POSITIVE cannot arm the pace band, and that guard
// is not paranoia: the band means "spending a little fast", and a matrix whose
// conserve block steps up would otherwise answer it by spending faster still.
func armEffortStep(m *Matrix, mode Mode, cap Capacity) effortIntent {
	if m == nil {
		return effortIntent{}
	}
	switch mode {
	case ModeConserve, ModeSpendDown:
		s, ok := m.EffortStepFor(mode)
		if !ok || s.EffortStep == 0 {
			return effortIntent{}
		}
		return effortIntent{
			armed:   true,
			notches: s.EffortStep,
			mode:    mode,
			shift:   s,
			band:    fmt.Sprintf("the decision is %s", mode),
		}
	}

	// NORMAL. The only thing that arms a step here is the lower overspend band,
	// read from the SAME pace verdict the mode was decided against.
	if cap.Pace == nil || !cap.Pace.BlocksSpendDown() || cap.Pace.Conserves() {
		return effortIntent{}
	}
	s, ok := m.EffortStepFor(ModeConserve)
	if !ok || s.EffortStep >= 0 {
		return effortIntent{}
	}
	return effortIntent{
		armed:   true,
		notches: s.EffortStep,
		mode:    ModeConserve,
		shift:   s,
		band: fmt.Sprintf(
			"%s is running at %.2fx the expected share of the %s window — past the lower band (block_spend_down_at_ratio) and not yet at conserve_at_ratio, so the capability is left exactly where it was and only the thinking time is trimmed",
			cap.Provider, cap.Pace.Ratio, cap.Pace.Window),
	}
}

// applyEffortStep is the second half: move the landing assignment's effort by
// the armed number of notches, and say what happened in one sentence.
//
// EVERY ARM WRITES EXACTLY ONE REASON LINE AND SETS d.EffortStep. A step that
// was armed and then refused is a fact worth publishing — an operator who wrote
// `effort_step: -1` and sees no trim needs to learn that the row declares no
// effort, or that its `min_effort` floor is already the answer, rather than
// concluding the knob does nothing.
//
// It reads the assignment rather than d.Effort so the floor and the ladder come
// from the row that actually won: after a fallover that is the ALTERNATIVE's
// own row, which carries its own effort and its own `min_effort`.
func applyEffortStep(m *Matrix, d *Decision, a Assignment, intent effortIntent) {
	if !intent.armed {
		return
	}
	step := func(to, why string) {
		d.EffortStep = &EffortStep{From: a.Effort, To: to, Why: why}
		d.Effort = to
		d.Reason = append(d.Reason, why)
	}
	direction := "down"
	if intent.notches > 0 {
		direction = "up"
	}
	head := fmt.Sprintf("mode_shifts.%s.effort_step is %+d, and %s", intent.mode, intent.notches, intent.band)

	// The allow-list is judged against the decision's FINAL capability — after
	// the mode shift and after the ceiling — because the capability that is
	// about to run is the one whose thinking time is being traded away.

	if !intent.shift.StepsCapability(d.Capability) {
		step(a.Effort, fmt.Sprintf(
			"%s — but capability %s is not in mode_shifts.%s.effort_step_capabilities, so its effort is left at %s. Thinking time is the expensive part of a frontier tier and a small part of a cheap one: a cheaper scout is mostly a worse scout",
			head, d.Capability, intent.mode, describeEffort(a.Effort)))
		return
	}
	if strings.TrimSpace(a.Effort) == "" {
		step("", fmt.Sprintf(
			"%s — but %s %s declares no effort at all, so it runs at the provider's own default and there is no rung to step %s from. Write an `effort:` on that row to make it steppable",
			head, a.Provider, a.Model, direction))
		return
	}
	ladder, ok := EffortLadder(a.Provider)
	if !ok {
		step(a.Effort, fmt.Sprintf(
			"%s — but %s publishes no effort ladder this router knows, so `%s` cannot be counted from and the effort is left as declared",
			head, a.Provider, a.Effort))
		return
	}
	rung, ok := effortRung(a.Provider, a.Effort)
	if !ok {
		step(a.Effort, fmt.Sprintf(
			"%s — but effort %q is not on %s's ladder (%s), so there is no rung to step from. ValidateAgainstCatalog reports the same row against the installed CLI",
			head, a.Effort, a.Provider, strings.Join(ladder, ", ")))
		return
	}
	if promotedAlready(m, d, intent) {
		step(a.Effort, fmt.Sprintf(
			"%s — but %s has already moved this role UP a capability tier, from %s to %s, and one promotion per decision is the rule: the effort stays at the `%s` that tier's own row declares rather than being raised a second time",
			head, intent.mode, d.BaseCapability, d.Capability, a.Effort))
		return
	}

	target := rung + intent.notches
	clamped := ""
	switch {
	case target < 0:
		target = 0
		clamped = fmt.Sprintf(" (clamped at %s's floor `%s` — the ladder runs out before the step does)", a.Provider, ladder[0])
	case target > len(ladder)-1:
		target = len(ladder) - 1
		clamped = fmt.Sprintf(" (clamped at %s's ceiling `%s` — the ladder runs out before the step does)", a.Provider, ladder[len(ladder)-1])
	}
	floored := ""
	if floor, ok := effortRung(a.Provider, a.MinEffort); ok && target < floor {
		target = floor
		floored = fmt.Sprintf(" (held at the row's own min_effort floor `%s`)", ladder[floor])
	}
	to := ladder[target]
	if to == strings.ToLower(strings.TrimSpace(a.Effort)) {
		step(a.Effort, fmt.Sprintf(
			"%s — so %s %s would step %s from `%s`, and it stays there%s%s",
			head, a.Provider, a.Model, direction, a.Effort, clamped, floored))
		return
	}
	step(to, fmt.Sprintf(
		"%s — so %s %s steps %s the %s ladder from `%s` to `%s`%s%s. That trims how long the SAME model thinks; it does not change which model runs the work",
		head, a.Provider, a.Model, direction, a.Provider, a.Effort, to, clamped, floored))
}

// promotedAlready is the spend-down cap: a decision this mode has ALREADY moved
// up a capability tier keeps that tier's own declared effort rather than being
// raised a second time.
//
// It compares CAPABILITY RANKS rather than trusting that a shift fired, because
// the ceiling runs between the two (step 7b) and routinely takes the promotion
// straight back: the shipped `default` ceiling caps at frontier, so a spend_down
// implementer is moved to frontier_max and clamped to frontier in the same
// decision. Nothing was promoted there, so nothing is capped — reading the shift
// alone would have made the step unreachable in exactly the configuration this
// file ships with.
//
// An UNRANKED capability on either side answers "no promotion": the ceiling
// already fails closed on those, and inventing an ordering here would be a
// second, quieter ladder disagreeing with `capability_ranks:`.
func promotedAlready(m *Matrix, d *Decision, intent effortIntent) bool {
	if intent.mode != ModeSpendDown {
		return false
	}
	landed, base := m.RankOf(d.Capability), m.RankOf(d.BaseCapability)
	if landed == UnrankedCapability || base == UnrankedCapability {
		return false
	}
	return landed > base
}

func describeEffort(effort string) string {
	if strings.TrimSpace(effort) == "" {
		return "the provider's own default"
	}
	return "`" + effort + "`"
}

// validateEffortStepping is the load-time half, and it reports rather than
// refuses, like every other Issue in this package.
//
// It covers all three new knobs, because a knob that is parsed and never
// enforced looks exactly like a working one:
//
//	effort_step               a step in the wrong DIRECTION for its mode is
//	                          almost certainly a sign error, and it is the one
//	                          mistake that spends more allowance in the state
//	                          that has least.
//	effort_step_capabilities  a capability that is not in the `capabilities:`
//	                          list can never match, so an allow-list of typos
//	                          silently means "step nothing".
//	min_effort                a floor that is not on the provider's own ladder
//	                          can never be compared, and a floor ABOVE the
//	                          effort the row declares is a contradiction the
//	                          router resolves in the floor's favour.
func validateEffortStepping(m *Matrix) []Issue {
	var issues []Issue
	add := func(where, format string, args ...any) {
		issues = append(issues, Issue{Where: where, Detail: fmt.Sprintf(format, args...)})
	}
	declared := map[string]bool{}
	for _, c := range m.Capabilities {
		declared[c] = true
	}

	for _, mode := range sortedKeys(m.ModeShifts) {
		shift := m.ModeShifts[mode]
		where := "mode_shifts." + mode
		switch {
		case shift.EffortStep > 0 && mode == string(ModeConserve):
			add(where, "effort_step is %+d — a POSITIVE step under conserve asks a constrained provider to think LONGER, which spends the allowance faster in exactly the state that has least of it. Conserve steps down (-1)", shift.EffortStep)
		case shift.EffortStep < 0 && mode == string(ModeSpendDown):
			add(where, "effort_step is %+d — a NEGATIVE step under spend_down trims thinking time on capacity that is about to expire unused, which is the opposite of what spending down is for", shift.EffortStep)
		}
		for _, c := range shift.EffortStepCapabilities {
			if !declared[strings.ToLower(strings.TrimSpace(c))] {
				add(where+".effort_step_capabilities", "capability %q is not in the `capabilities:` list, so this mode's effort step can never apply to it", c)
			}
		}
	}

	checkFloor := func(a Assignment, where string) {
		if strings.TrimSpace(a.MinEffort) == "" {
			return
		}
		ladder, ok := EffortLadder(a.Provider)
		if !ok {
			add(where, "min_effort %q is written for provider %q, which publishes no effort ladder this router knows, so the floor can never be compared with anything", a.MinEffort, a.Provider)
			return
		}
		floor, ok := effortRung(a.Provider, a.MinEffort)
		if !ok {
			add(where, "min_effort %q is not on %s's effort ladder (%s), so nothing can be held above it", a.MinEffort, a.Provider, strings.Join(ladder, ", "))
			return
		}
		if strings.TrimSpace(a.Effort) == "" {
			return
		}
		rung, ok := effortRung(a.Provider, a.Effort)
		if !ok {
			return // the effort itself is wrong; ValidateAgainstCatalog says so
		}
		if rung < floor {
			add(where, "min_effort %q is ABOVE the effort %q this row declares, so an effort step could only ever raise this row — write the floor at or below the declared effort", a.MinEffort, a.Effort)
		}
	}
	for _, pname := range sortedKeys(m.Profiles) {
		prof := m.Profiles[pname]
		for _, capability := range sortedKeys(prof) {
			a := prof[capability]
			checkFloor(a, "profiles."+pname+"."+capability)
			for i, alt := range a.Alternatives {
				checkFloor(alt, alternativePath(pname, capability, i))
			}
		}
	}
	return issues
}
