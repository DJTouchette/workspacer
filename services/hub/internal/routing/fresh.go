package routing

// FRESHNESS, AS A VERDICT. This file turns `fresh: true` in the profile table
// from something reported on a routing decision into something the spawn gate
// refuses to violate.
//
// WHAT `fresh` MEANS. A reviewer must not inherit the implementer's
// conversation: it gets the ticket, the acceptance criteria, the diff, the
// relevant source and the test results, and not the reasoning history of the
// agent whose work it is grading. In a single-family profile that is the ONLY
// thing making the reviewer independent at all, which is why every shipped
// profile sets it on reviewer, deep_reviewer and frontier_plus.
//
// THE HOLE THIS CLOSES. `resumeSessionId` is on the spawn wire, and a spawn
// carrying one CONTINUES an existing session rather than starting one. So a
// dispatch could point a reviewer at the implementer's own session and the
// reviewer would inherit the exact reasoning chain it exists to grade
// independently. The flag was on the decision, the decision was on the wire, and
// nothing anywhere refused the resume. A setting written and never read.
//
// WHY THE DECLARED ROLE IS ENOUGH, and why this is NOT the ceiling's problem.
// The capability ceiling refuses to trust `decisionId` because provenance is
// forgeable and a caller could talk its way UP to more authority. Freshness is
// not an authority axis: refusing a resume only ever gives the caller LESS
// continuity, and a caller who lies about its role to obtain a resume gains
// nothing it could not have had by declaring no role at all. So this arm keys
// off what the spawn DECLARES — `role`, `capability` — and there is deliberately
// no provenance mechanism behind it. Building one would cost a forgeable field a
// second implementation and buy nothing.
//
// AND WHY THERE IS NO NAMED-MODEL ARM. The ceiling has one because a caller can
// walk around a capability LABEL by naming a reserved model directly. Freshness
// has no equivalent: `model: sonnet, effort: high` is what `reviewer` resolves
// to AND what ordinary balanced work resolves to in the anthropic_only profile,
// so judging the model would refuse resumes for work that was never a review.
// The label is the whole claim here, because the label is what a dispatch sets
// when it means "this is a review".

import (
	"fmt"
	"strings"
)

// checkFresh refuses a spawn that asks to RESUME a session while declaring a
// role or capability the matrix marks `fresh: true`.
//
// It runs on every spawn, whether or not a `ceilings:` row governs the
// directory: freshness is a property of the WORK, not of where it happens, and a
// matrix with no ceilings block at all must still keep a reviewer independent.
//
// ABSENCE IS NOT A VIOLATION. A spawn that declares no role and no capability
// makes no freshness claim, so a resume is ordinary and is allowed. A role or
// capability the matrix has never heard of is the same answer for the same
// reason: the matrix expresses no opinion, and failing closed on an unknown
// label would refuse resumes for every worker kind this file does not name while
// buying nothing — the caller could simply have omitted the label.
func (m *Matrix) checkFresh(req SpawnRequest, v *CeilingVerdict) {
	if m == nil || !req.Resuming {
		return
	}
	capability, reading, ok := m.freshRequirement(req.Role, req.Capability)
	if !ok {
		return
	}
	v.ResumeRefused, v.FreshCapability = true, capability
	v.Because = append(v.Because, fmt.Sprintf(
		"this spawn asked to RESUME session %s while declaring %s, and routing.yaml marks capability %s `fresh: true`. A worker that inherits the previous agent's conversation inherits the reasoning it was spawned to judge independently, so the resume is REFUSED rather than quietly turned into a new session. Spawn it without `resumeSessionId` and hand it the ticket, the diff and the test results instead; if this work genuinely is a continuation of what came before, it is not %s work",
		req.ResumeSessionID, reading, capability, capability))
}

// freshRequirement answers which capability, if any, makes this spawn's declared
// work fresh — and which reading of the declaration got there, for the sentence.
//
// IT READS THE DECLARATION AT ITS STRONGEST, the same discipline the ceiling's
// model arm uses. Three readings are considered, in this order:
//
//  1. the declared `capability`, resolved directly;
//  2. the capability the declared `role` asks for under `roles:`;
//  3. the capability any MODE SHIFT would move that role to.
//
// The third is the one worth explaining. A mode shift moves a role's capability
// (`spend_down` promotes the reviewer role from `reviewer` to `deep_reviewer`;
// `conserve` moves the judge from `frontier_plus` down to `deep_reviewer`), and
// the gate does not know which mode was in force when the caller made its
// decision, nor whether the mode has changed since. So a role whose capability is
// fresh under ANY reading the matrix has for it is refused a resume. That costs
// nothing in the shipped file, where both readings agree for every role that has
// a shift, and it means a hand-edited matrix cannot quietly make a reviewer
// resumable by adding a mode shift onto a non-fresh capability.
//
// The direction is also what makes the strongest reading the cheap one here. The
// worst case is a resume refused for work that did not need to be fresh, which
// costs continuity; the other direction costs the independence of a review.
func (m *Matrix) freshRequirement(role, capability string) (string, string, bool) {
	type reading struct{ capability, how string }
	var readings []reading

	if c := strings.ToLower(strings.TrimSpace(capability)); c != "" {
		readings = append(readings, reading{c, fmt.Sprintf("capability %s", c)})
	}
	if r := strings.TrimSpace(role); r != "" {
		if base, ok := m.roleCapability(r); ok {
			readings = append(readings, reading{base, fmt.Sprintf("role %s", r)})
		}
		for _, mode := range sortedKeys(m.ModeShifts) {
			if shifted, ok := m.ShiftFor(mode, r); ok {
				readings = append(readings, reading{
					strings.ToLower(strings.TrimSpace(shifted)),
					fmt.Sprintf("role %s, which mode_shifts.%s moves to %s", r, mode, shifted),
				})
			}
		}
	}

	for _, rd := range readings {
		if a, ok := m.freshAssignment(rd.capability); ok && a.Fresh {
			return rd.capability, rd.how, true
		}
	}
	return "", "", false
}

// roleCapability is the capability a role asks for, falling back to the shipped
// default's answer the way ResolveRole does — a user document that omits a role
// has not deleted it, because a deep merge cannot delete.
func (m *Matrix) roleCapability(role string) (string, bool) {
	if c, ok := m.Capability(role); ok {
		return strings.ToLower(strings.TrimSpace(c)), true
	}
	if m.fallback != nil {
		if c, ok := m.fallback.Capability(role); ok {
			return strings.ToLower(strings.TrimSpace(c)), true
		}
	}
	return "", false
}

// freshAssignment resolves a capability under the ACTIVE profile, because the
// active profile is what this spawn's work would actually resolve through.
//
// Reading every profile instead would let a `fresh: true` in a profile nobody
// has selected refuse a resume the selected profile permits, which would make
// the flag mean something other than what the file says. Editing `fresh: false`
// on the active profile's reviewer entry is a real decision the user is entitled
// to make, and it is honoured.
func (m *Matrix) freshAssignment(capability string) (Assignment, bool) {
	profile, _ := m.ActiveProfileName()
	if a, err := m.ResolveCapability(profile, capability); err == nil {
		return a, true
	}
	if m.fallback == nil {
		return Assignment{}, false
	}
	if a, err := m.fallback.ResolveCapability(profile, capability); err == nil {
		return a, true
	}
	fbProfile, _ := m.fallback.ActiveProfileName()
	if a, err := m.fallback.ResolveCapability(fbProfile, capability); err == nil {
		return a, true
	}
	return Assignment{}, false
}
