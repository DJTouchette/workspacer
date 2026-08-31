package routing

import (
	"strings"
	"testing"
)

// THE GUARANTEE `fresh` EXISTS TO MAKE. Every test here asks CheckSpawn the same
// question the spawn gate asks it: may this spawn inherit a conversation?

// loadMatrix is Load with the error handled, for the hand-written documents
// below.
func loadMatrix(t *testing.T, yaml string) *Matrix {
	t.Helper()
	m, err := Load("routing.yaml", []byte(yaml))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	return m
}

func resume(role, capability string) SpawnRequest {
	return SpawnRequest{
		CanonicalCwd:    "/home/someone/project",
		Role:            role,
		Capability:      capability,
		Resuming:        true,
		ResumeSessionID: "implementer-session-1",
	}
}

// THE HEADLINE: the three roles the shipped matrix marks fresh cannot be pointed
// at an existing session, in any profile.
func TestAFreshRoleMayNotResumeASession(t *testing.T) {
	m := shippedMatrix(t)
	for _, profile := range sortedKeys(m.Profiles) {
		m.ActiveProfile = profile
		for _, role := range []string{"reviewer", "deep_reviewer", "judge"} {
			v := m.CheckSpawn(resume(role, ""))
			if !v.ResumeRefused {
				t.Errorf("profile %s: role %s was allowed to resume a session — a reviewer that inherits the implementer's conversation inherits the reasoning it is grading", profile, role)
				continue
			}
			if !v.Refused() {
				t.Errorf("profile %s: role %s: a resume refusal does not count as a refusal", profile, role)
			}
			if v.FreshCapability == "" {
				t.Errorf("profile %s: role %s: the refusal does not name the entry that refused it: %+v", profile, role, v)
			}
			// The sentence is what the caller sees, so it has to carry the id it
			// asked for and the way out.
			why := strings.Join(v.Because, " | ")
			for _, want := range []string{"implementer-session-1", "fresh", "resumeSessionId"} {
				if !strings.Contains(why, want) {
					t.Errorf("profile %s: role %s: the refusal does not mention %q: %s", profile, role, want, why)
				}
			}
		}
	}
}

// A capability declared DIRECTLY, with no role, resolves the same way. A routed
// dispatch copies the decision's `capability` onto the spawn and may name no
// role at all, so a rule that only read `role` would be walked around by the
// most ordinary caller there is.
func TestAFreshCapabilityDeclaredWithoutARoleIsAlsoRefused(t *testing.T) {
	m := shippedMatrix(t)
	for _, capability := range []string{"reviewer", "deep_reviewer", "frontier_plus"} {
		if v := m.CheckSpawn(resume("", capability)); !v.ResumeRefused {
			t.Errorf("capability %s resumed a session with no role declared: %+v", capability, v)
		}
	}
}

// NOT EVERY SPAWN IS A REVIEW. Without this the refusal could be firing on
// everything and the tests above would not notice.
func TestOrdinaryWorkKeepsItsResume(t *testing.T) {
	m := shippedMatrix(t)
	cases := []struct{ role, capability string }{
		{"", ""},                    // declares nothing: no freshness claim
		{"implementer", ""},         // frontier, not marked fresh
		{"scout", ""},               // balanced
		{"", "frontier"},            // a capability, declared directly
		{"", "cheap"},               //
		{"implementer", "frontier"}, // both, agreeing
		{"nobody-defined-this", ""}, // a role the matrix has never heard of
		{"", "nobody-ranked-this"},  // ditto for a capability
	}
	for _, c := range cases {
		if v := m.CheckSpawn(resume(c.role, c.capability)); v.ResumeRefused {
			t.Errorf("role %q / capability %q was refused a resume it is entitled to: %s",
				c.role, c.capability, strings.Join(v.Because, " | "))
		}
	}
}

// A SPAWN THAT IS NOT RESUMING IS NEVER TOUCHED BY THIS ARM. Freshness is a
// refusal to inherit, not a restriction on reviewers.
func TestAFreshRoleStartingANewSessionIsFine(t *testing.T) {
	m := shippedMatrix(t)
	for _, role := range []string{"reviewer", "deep_reviewer", "judge"} {
		req := resume(role, "")
		req.Resuming, req.ResumeSessionID = false, ""
		if v := m.CheckSpawn(req); v.ResumeRefused {
			t.Errorf("role %s was refused while starting a NEW session: %+v", role, v)
		}
	}
}

// FRESHNESS DOES NOT DEPEND ON THE CEILING. CheckSpawn returns early when no
// `ceilings:` row governs the directory, and a reviewer's independence has
// nothing to do with whether anyone capped the tree it works in.
func TestFreshIsEnforcedWithNoCeilingsBlockAtAll(t *testing.T) {
	m := shippedMatrix(t)
	m.Ceilings = nil
	m.fallback = nil // no shipped `default:` to fall back onto either
	v := m.CheckSpawn(resume("reviewer", ""))
	if v.Key != "" {
		t.Fatalf("this matrix was supposed to have no ceiling at all: %+v", v)
	}
	if !v.ResumeRefused {
		t.Error("with no ceilings block, the freshness refusal stopped firing — it was riding on the ceiling's early return")
	}
}

// THE MODE-SHIFT READING. `spend_down` moves the reviewer role onto
// deep_reviewer and `conserve` moves the judge down onto deep_reviewer, so the
// gate has more than one reading of what a role's capability is and does not
// know which mode the caller decided under. It refuses if ANY reading is fresh.
//
// The document below makes the two readings DISAGREE, which the shipped file
// never does: the base capability is not fresh and the shifted one is.
func TestAModeShiftOntoAFreshCapabilityStillRefusesTheResume(t *testing.T) {
	m := loadMatrix(t, `
active_profile: mixed
roles:
  drifter: balanced
mode_shifts:
  spend_down:
    drifter: deep_reviewer
`)
	v := m.CheckSpawn(resume("drifter", ""))
	if !v.ResumeRefused {
		t.Fatalf("a role a mode shift moves onto a fresh capability kept its resume: %+v", v)
	}
	if v.FreshCapability != "deep_reviewer" {
		t.Errorf("the refusal names %q, want deep_reviewer", v.FreshCapability)
	}
	if !strings.Contains(strings.Join(v.Because, " | "), "mode_shifts.spend_down") {
		t.Errorf("the refusal does not say which reading refused it: %s", strings.Join(v.Because, " | "))
	}
}

// AND THE OTHER DIRECTION. A role whose BASE capability is fresh keeps the
// refusal even when a mode shift would move it onto something that is not, so a
// mode shift cannot be the quiet way to make a reviewer resumable.
func TestAModeShiftAwayFromFreshDoesNotRestoreTheResume(t *testing.T) {
	m := loadMatrix(t, `
active_profile: mixed
roles:
  auditor: deep_reviewer
mode_shifts:
  conserve:
    auditor: balanced
`)
	if v := m.CheckSpawn(resume("auditor", "")); !v.ResumeRefused {
		t.Errorf("a mode shift onto a non-fresh capability handed a fresh role its resume back: %+v", v)
	}
}

// THE ACTIVE PROFILE IS THE ONE THAT GOVERNS, and `fresh: false` on it is a real
// decision the user is entitled to make. Reading every profile instead would let
// a flag in a profile nobody selected refuse a resume the selected one permits,
// which would make the file say something other than what it does.
func TestTheActiveProfilesOwnFlagIsWhatDecides(t *testing.T) {
	m := loadMatrix(t, `
active_profile: anthropic_only
profiles:
  anthropic_only:
    reviewer: { provider: claude, model: sonnet, effort: high, fresh: false }
`)
	// The capability is declared DIRECTLY rather than through the role, because
	// `mode_shifts.spend_down` moves the reviewer role onto deep_reviewer and
	// that entry is still fresh — which is the strongest-reading rule doing its
	// job, and a different claim from the one under test here.
	if v := m.CheckSpawn(resume("", "reviewer")); v.ResumeRefused {
		t.Errorf("`fresh: false` on the active profile's reviewer was overruled: %s", strings.Join(v.Because, " | "))
	}
	// The same matrix under a profile that DID keep the flag still refuses, so
	// the case above is a reading of the file rather than the arm going quiet.
	m.ActiveProfile = "codex_only"
	if v := m.CheckSpawn(resume("", "reviewer")); !v.ResumeRefused {
		t.Errorf("switching to a profile that keeps `fresh: true` did not restore the refusal: %+v", v)
	}
}

// THE TWO REFUSALS ARE INDEPENDENT and both are reported. A spawn can be over
// the ceiling AND asking to inherit a review's session, and a caller that fixed
// only the half it was told about would come straight back.
func TestACeilingClampAndAFreshRefusalAreBothExplained(t *testing.T) {
	m := loadMatrix(t, `
ceilings:
  /home/someone/project: { max_capability: cheap, max_tool_scope: view }
`)
	req := resume("", "deep_reviewer")
	req.ToolScope = "operator"
	v := m.CheckSpawn(req)
	if !v.ResumeRefused || !v.CapabilityRefused || !v.ToolScopeRefused {
		t.Fatalf("the three arms did not all fire: %+v", v)
	}
	why := strings.Join(v.Because, " | ")
	if !strings.Contains(why, "ceilings./home/someone/project") || !strings.Contains(why, "resumeSessionId") {
		t.Errorf("the explanation drops one of the two reasons: %s", why)
	}
}

// Select never sets this arm: it answers a question about a model, and nothing
// there is resuming anything. Pinned so the shared CheckSpawn cannot start
// refusing routing.select's own advice.
func TestSelectNeverProducesAResumeRefusal(t *testing.T) {
	m := shippedMatrix(t)
	for _, role := range sortedKeys(m.Roles) {
		v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/home/someone/project", Role: role})
		if v.ResumeRefused {
			t.Errorf("role %s: a non-resuming request was refused: %+v", role, v)
		}
	}
}
