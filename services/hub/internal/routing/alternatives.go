package routing

// PER-CAPABILITY ALTERNATIVES: the same capability, a different provider.
//
// A capability entry may carry an ORDERED list of `alternatives:` (see
// Assignment.Alternatives). When the primary pairing cannot be used, the router
// walks that list and takes the first candidate that can be, and says so.
//
// THIS IS NOT A MODE SHIFT AND THE TWO MUST NOT BE CONFLATED. A `mode_shifts:`
// entry moves a ROLE onto a DIFFERENT CAPABILITY — a conserving scout drops from
// balanced to cheap — and which provider that lands on is a consequence, not the
// point. A fallover keeps the CAPABILITY EXACTLY AS IT IS and changes only which
// provider serves it: the work is worth the same, and the machine that was going
// to do it cannot right now. They compose in that order, and the order is the
// one Select already documents: the mode moves the capability (step 7), the
// ceiling caps the result (step 7b), and only then is the capability resolved to
// a pairing (step 8), which is where this file runs.
//
// THE WALK IS SCOPED TO ONE CAPABILITY'S OWN LIST. It never searches other
// profiles and never invents a pairing: the candidates are exactly the primary
// and the alternatives the profile author wrote, in the order they wrote them.
// A profile that lists none behaves exactly as it did before this file existed.
//
// IT IS PURE, like the rest of policy.go. Every judgement below is made from the
// matrix, from the limits.Snapshot that was passed in, and from the
// ProviderAvailability map that was passed in. There is still no live probe
// INSIDE this file: the load-time catalog validation's findings ride
// Matrix.Issues, and the LIVE half — whether a provider's CLI answers at all
// right now — is read where the catalog is already fetched (cmd/hub's
// routingCatalog) and injected as an argument, which is the shape this comment
// used to say such a feature belonged in. See availability.go, and note the
// fail-open rule: a provider nobody could ask about is UNKNOWN and routes
// exactly as it did before.

import (
	"fmt"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// candidate is one pairing in a capability's own list, with the position that
// names it in the document.
type candidate struct {
	a Assignment
	// index is -1 for the primary and 0..n-1 for Alternatives[index]. It is
	// what builds the Issues path, so it has to be the document's index rather
	// than the walk's — reordering for independence must not renumber a row.
	index int
}

// primary reports whether this candidate is the capability's own primary entry.
func (c candidate) primary() bool { return c.index < 0 }

// describe names a candidate the way a reason sentence does.
func (c candidate) describe() string {
	return fmt.Sprintf("%s %s%s", c.a.Provider, c.a.Model, effortSuffix(c.a.Effort))
}

// providerJudge answers "can work go to this provider right now" for a whole
// walk, reading each provider's OWN capacity at most once.
//
// The memo is not an optimization, or not only one: two candidates on the same
// provider must be judged identically within a single decision, and a second
// reading taken microseconds later against the same snapshot would be the same
// answer arrived at twice. One reading, quoted twice.
type providerJudge struct {
	m       *Matrix
	snap    limits.Snapshot
	snapErr error
	avail   ProviderAvailability
	now     time.Time
	account string
	demand  limits.Demand
	seen    map[string]judged
}

type judged struct {
	capacity Capacity
	mode     Mode
}

func newProviderJudge(m *Matrix, snap limits.Snapshot, snapErr error, avail ProviderAvailability, now time.Time, account string, demand limits.Demand) *providerJudge {
	return &providerJudge{m: m, snap: snap, snapErr: snapErr, avail: avail, now: now, account: account, demand: demand, seen: map[string]judged{}}
}

// judge reads a provider's capacity and the mode its OWN capacity decides.
//
// Reading the candidate's own capacity rather than inheriting the subject's is
// the whole discipline this file borrows from applyShift: health does not
// transfer across providers, and "codex is red, so use claude" is only an
// argument if somebody looked at claude.
func (j *providerJudge) judge(provider string) judged {
	provider = normalizeProvider(provider)
	if got, ok := j.seen[provider]; ok {
		return got
	}
	cap := capacityFor(j.m, j.snap, j.snapErr, provider, j.account, j.now)
	verdict := DecideMode(cap, j.demand, j.m.Thresholds, j.m.ModeFor(provider))
	got := judged{capacity: cap, mode: verdict.Mode}
	j.seen[provider] = got
	return got
}

// unusable is the trigger inventory, and it is the whole behavioural surface of
// this feature. It returns the sentence naming WHY, and "" when the candidate
// can be routed to.
//
// The order is cheapest-and-most-definite first, so a decision quotes the reason
// an operator can act on rather than a consequence of it: a provider held out of
// service is not "red", it is switched off.
//
//	enabled: false           on the entry itself. `enabled:` is documented as the
//	                         one spelling for taking something out of service; a
//	                         candidate carrying it is not a candidate.
//	no provider or model      an entry that names neither is nothing to spawn.
//	provider enabled: false   the provider-level flag, same rule as step 3b's.
//	a load-time Issue         validate() or the catalog check flagged THIS row —
//	                          an unknown provider id, a model the installed CLI
//	                          does not serve, an effort it does not take. Routing
//	                          onto a row the loader already condemned is how a
//	                          fallover fails at the exact moment it is needed.
//	provider UNAVAILABLE      the LIVE reading: this provider's CLI RAN and
//	                          reported no launchable model. It is judged BEFORE
//	                          health because a provider that cannot start
//	                          anything is not usefully described as green. A
//	                          provider nobody could ask about, and one whose
//	                          probe failed, are both UNKNOWN and both pass — see
//	                          availability.go's fail-open rule, which is also
//	                          where the limits of this check are written down.
//	                          Note the two checks are different scopes and both
//	                          stay: this one is the PROVIDER, the Issue above is
//	                          the specific MODEL on it, so a candidate on a live
//	                          provider whose own model is flagged is still
//	                          unusable.
//	RED or EXHAUSTED          the allowance is gone or nearly gone.
//	CONSERVE                  the provider's own mode verdict, which also covers
//	                          being over the window-progress curve and being
//	                          over-forecast. It is read from the CANDIDATE's
//	                          capacity, never from the subject's.
func (j *providerJudge) unusable(c candidate, path string) string {
	if !c.a.IsEnabled() {
		return "the entry is explicitly disabled (enabled: false)"
	}
	provider := normalizeProvider(c.a.Provider)
	if provider == "" || strings.TrimSpace(c.a.Model) == "" {
		return "the entry names no provider and model to spawn"
	}
	if _, off := providerDisabled(j.m, provider); off {
		return fmt.Sprintf("routing.yaml's providers.%s is explicitly disabled (enabled: false)", provider)
	}
	for _, iss := range j.m.Issues {
		if iss.Where == path {
			return fmt.Sprintf("the matrix's load-time validation flags %s — %s", path, iss.Detail)
		}
	}
	if why, off := j.avail.Unusable(provider); off {
		return why
	}
	got := j.judge(provider)
	switch got.capacity.EffectiveHealth {
	case limits.HealthRed, limits.HealthExhausted:
		return fmt.Sprintf("%s's allowance is %s: %s", provider, got.capacity.EffectiveHealth, got.capacity.Because)
	}
	if got.mode == ModeConserve {
		return fmt.Sprintf("%s is in CONSERVE on its own capacity: %s", provider, got.capacity.Because)
	}
	return ""
}

// candidatesFor is a capability entry read as its ordered candidate list.
func candidatesFor(primary Assignment) []candidate {
	out := make([]candidate, 0, 1+len(primary.Alternatives))
	out = append(out, candidate{a: primary, index: -1})
	for i, alt := range primary.Alternatives {
		out = append(out, candidate{a: alt, index: i})
	}
	return out
}

// preferIndependent is decision 2 in one function: family independence is a
// PREFERENCE the router tries to honour, expressed as an ORDER rather than as a
// refusal.
//
// It moves every candidate on a different provider from the previous agent's
// ahead of the ones that share it, stably, so the file's own order survives
// inside each group. The usability filter then runs over the reordered list, so
// an independent candidate that is itself red loses to a healthy same-family one
// — the preference is real and it is not absolute, which is exactly what the
// user asked for and what `fresh: true` (which stays hard) is there to backstop.
//
// It reorders THIS CAPABILITY'S OWN LIST and nothing else. It does not search
// other profiles the way a pinned provider does: a reviewer landing on a pairing
// some unrelated profile happens to spell would be a re-route nobody wrote down.
func preferIndependent(cands []candidate, previous string) ([]candidate, bool) {
	prev := normalizeProvider(previous)
	if prev == "" || len(cands) < 2 {
		return cands, false
	}
	independent := make([]candidate, 0, len(cands))
	same := make([]candidate, 0, len(cands))
	for _, c := range cands {
		if normalizeProvider(c.a.Provider) != prev {
			independent = append(independent, c)
		} else {
			same = append(same, c)
		}
	}
	if len(independent) == 0 || len(same) == 0 {
		return cands, false // nothing to prefer: the list is all one way
	}
	out := append(independent, same...)
	moved := false
	for i := range out {
		if out[i].index != cands[i].index {
			moved = true
			break
		}
	}
	return out, moved
}

// walkAlternatives is step 8's fallover: resolve a capability to the first
// candidate in its own list that can actually be used, and explain the walk.
//
// It runs AFTER the ceiling has clamped the capability (step 7b), because the
// question it answers is "who serves THIS capability", and the ceiling decides
// which capability that is. It runs only for an UNCONSTRAINED request: a caller
// that named a provider asked about that provider, and §32 is explicit that a
// model on a provider nobody asked for is not an answer to that question. A
// pinned provider is served from this same list by assignmentFor, which now
// looks at the alternatives before borrowing another profile's pairing.
//
// NOTHING USABLE MEANS THE PRIMARY, not a refusal. Every candidate being
// unusable is precisely the state routing was in before this feature existed —
// the matrix's answer, with the capacity picture and the reasons attached — and
// answering it identically is what keeps the fallover a strict addition. The
// later arms of Select (`enabled: false`, the provider check) still judge what
// comes back, so a walk that ends on a disabled primary still refuses.
func (d *Decision) walkAlternatives(
	m *Matrix, snap limits.Snapshot, snapErr error, avail ProviderAvailability, now time.Time, req Request,
	profile string, primary Assignment,
) Assignment {
	if len(primary.Alternatives) == 0 {
		return primary
	}
	cands := candidatesFor(primary)
	if req.RequireIndependentFamily {
		if reordered, moved := preferIndependent(cands, req.PreviousProvider); moved {
			cands = reordered
			d.Reason = append(d.Reason, fmt.Sprintf(
				"the previous agent ran on %s and an independent family was asked for, so the candidates for %s were tried in the order that prefers a different one first — a PREFERENCE the walk honours where capacity allows, not a rule that refuses",
				normalizeProvider(req.PreviousProvider), d.Capability))
		}
	}

	judge := newProviderJudge(m, snap, snapErr, avail, now, req.wantedAccount(), d.Demand)
	pathOf := func(c candidate) string {
		if c.primary() {
			return "profiles." + profile + "." + d.Capability
		}
		return alternativePath(profile, d.Capability, c.index)
	}

	var (
		primaryWhy string
		skipped    []string
	)
	for _, c := range cands {
		why := judge.unusable(c, pathOf(c))
		if why == "" {
			if c.primary() {
				// The primary itself, either first (the ordinary answer,
				// reached the ordinary way) or after the independence
				// preference tried something ahead of it and that candidate
				// could not be used. Either way this is not a fallover.
				d.Reason = append(d.Reason, skipped...)
				return primary
			}
			from := primary
			from.Alternatives = nil
			d.FellOverFrom = &from
			// THE CANDIDATE'S OWN CAPACITY, KEPT RATHER THAN RE-READ. judge
			// already read it above (that is the whole discipline this walk
			// borrows from applyShift), so this is that same reading, quoted
			// into the fields a caller reads to learn what actually describes
			// the provider this answer is about to name — see
			// Decision.EffectiveCapacity. It OVERWRITES anything a mode shift
			// (step 7) left here: those fields always describe the provider
			// the CURRENT answer is prepared to defend, and after a fallover
			// that is this candidate, not wherever a mode shift landed before
			// the ceiling and this walk ran.
			landed := judge.judge(normalizeProvider(c.a.Provider))
			landedCapacity := landed.capacity
			d.ShiftCapacity, d.ShiftMode = &landedCapacity, landed.mode
			if primaryWhy != "" {
				d.Reason = append(d.Reason, fmt.Sprintf(
					"primary %s %s unusable (%s); took alternative %s %s",
					primary.Provider, primary.Model, primaryWhy, c.a.Provider, c.a.Model))
			} else {
				d.Reason = append(d.Reason, fmt.Sprintf(
					"took %s for capability %s ahead of the primary %s %s, which this walk was asked to move past rather than judged unusable",
					c.describe(), d.Capability, primary.Provider, primary.Model))
			}
			d.Reason = append(d.Reason, skipped...)
			return c.a
		}
		if c.primary() {
			primaryWhy = why
			continue
		}
		skipped = append(skipped, fmt.Sprintf(
			"alternative %s for capability %s was skipped too: %s", c.describe(), d.Capability, why))
	}

	// Nothing in the list can be used. Answer with the primary, exactly as this
	// layer did before alternatives existed, and say that the fallover was tried
	// and found nothing — a silent return here would read as "there was no
	// alternative", which is a different and untrue statement.
	if primaryWhy != "" {
		d.Reason = append(d.Reason, fmt.Sprintf(
			"primary %s %s for capability %s is unusable (%s) and so is every alternative, so the answer stays on the primary rather than moving to a provider that is no better",
			primary.Provider, primary.Model, d.Capability, primaryWhy))
	}
	d.Reason = append(d.Reason, skipped...)
	return primary
}
