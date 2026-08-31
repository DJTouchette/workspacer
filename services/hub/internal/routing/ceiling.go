package routing

// THE CEILING, AS A VERDICT. This file turns routing.yaml's `ceilings:` block
// into the single answer the enforcement site needs, so that internal/bus holds
// no matrix, no ladder and no vocabulary of its own — it holds a function it was
// handed at wiring time and applies what that function says.
//
// WHY THE ENFORCEMENT IS NOT HERE. The spawn path has exactly one piece of code
// in this repo that is not a twin — sanitizeSpawnParams in internal/bus — and it
// covers the federated hop by construction (methodSanitizers is the single
// dispatch table for call() and federatedCall()). Putting the clamp anywhere
// else means two copies of it. So the POLICY is here, where the matrix is, and
// the CLAMP is there, where every spawn already passes.
//
// WHY THE PATH MUST ARRIVE CANONICAL. CeilingFor is a LEXICAL ancestor match. It
// resolves nothing and opens nothing, which is correct for a pure function and
// fatal for a caller-supplied string: `/tmp/link -> /home/you/Work/secure` is a
// different key from the directory it names, so a ceiling looked up on the
// unresolved spelling is a ceiling a symlink walks straight around. The
// enforcement site canonicalizes first — the same check-path/opened-path rule
// the filesystem guard is built on — and CheckSpawn takes the canonical form as
// a documented precondition rather than re-deriving it, because a second
// canonicalizer is a second thing to disagree with the first.

import (
	"fmt"
	"strings"
)

// ToolScopeRank orders the AUTHORITY tiers. Unlike the capability ladder this is
// a closed, three-valued vocabulary defined by the security model
// (internal/authtoken), not by the matrix, so it is not configuration: a
// routing.yaml that could invent a fourth tier would be inventing a token scope.
func ToolScopeRank(scope string) (int, bool) {
	switch strings.ToLower(strings.TrimSpace(scope)) {
	case "view":
		return 1, true
	case "triage":
		return 2, true
	case "operator":
		return 3, true
	}
	return 0, false
}

// RankOf is a capability's strength on the matrix's own ladder.
// UnrankedCapability means the file does not rank it, which is a real answer and
// not zero — zero would read as "the weakest thing there is".
func (m *Matrix) RankOf(capability string) int {
	if m == nil {
		return UnrankedCapability
	}
	c := strings.ToLower(strings.TrimSpace(capability))
	if c == "" {
		return UnrankedCapability
	}
	if r, ok := m.CapabilityRanks[c]; ok {
		return r
	}
	if m.fallback != nil {
		if r, ok := m.fallback.CapabilityRanks[c]; ok {
			return r
		}
	}
	return UnrankedCapability
}

// SpawnRequest is what the enforcement site knows about one agents.spawn, in the
// only terms the ceiling cares about.
//
// CanonicalCwd is a PRECONDITION, not a hint. See the file header.
type SpawnRequest struct {
	CanonicalCwd string
	// Capability is the spawn's declared capability — the `capability` param,
	// which a routed dispatch copies from the decision it is acting on. Empty is
	// ordinary: nothing declared one before this feature existed.
	Capability string
	// ToolScope is the AUTHORITY tier the spawn asks for the child to hold, or
	// "operator" spelled as the legacy `mcpFacade: true`.
	ToolScope string
	// Provider / Model / Effort are what the spawn actually names. They exist
	// here for the arm that catches a spawn declaring a modest capability while
	// naming a reserved model — see CheckSpawn.
	Provider string
	Model    string
	Effort   string
}

// CeilingVerdict is the whole answer for one spawn: what the ceiling is, what it
// refuses, and the sentence the refusal is reported with.
//
// It is deliberately a CLAMP verdict and not a routing answer. It never names a
// model, never picks a provider and never promotes anything. Re-routing at the
// spawn gate would need the engine, a classification the caller did not supply,
// and a second place where model selection happens; lowering a ceiling needs one
// table lookup.
type CeilingVerdict struct {
	// Key is the `ceilings:` entry that matched — "default", an ancestor
	// directory, or "" when the matrix carries no ceiling at all.
	Key string `json:"key,omitempty"`
	// MaxCapability / MaxToolScope are that entry's limits, echoed so the log and
	// the caller-facing sentence quote the file rather than a paraphrase.
	MaxCapability string `json:"maxCapability,omitempty"`
	MaxToolScope  string `json:"maxToolScope,omitempty"`

	// CapabilityRefused is true when the spawn asks for more model capability
	// than this directory allows. Capability then holds what it is clamped TO.
	CapabilityRefused bool `json:"capabilityRefused,omitempty"`
	// ToolScopeRefused is true when the spawn asks for more AUTHORITY than this
	// directory allows. ToolScope then holds what it is clamped TO.
	ToolScopeRefused bool `json:"toolScopeRefused,omitempty"`

	// Capability / ToolScope are the clamped values, set only on the
	// corresponding Refused arm.
	Capability string `json:"capability,omitempty"`
	ToolScope  string `json:"toolScope,omitempty"`

	// Denied refuses the spawn OUTRIGHT rather than clamping it, and it is the
	// answer to the one question a clamp cannot express: what to do when the
	// CEILING ITSELF cannot be read.
	//
	// A ceiling row naming a capability the ranks block does not rank, or a tier
	// the security model does not have, used to be reported and then skipped —
	// so a typo in the policy file silently deleted the protection for that
	// directory, and the failure looked exactly like a directory that was never
	// capped. A ceiling nobody can parse must not become a ceiling nobody has.
	// The spawn stops, and the sentence in Because names the file, the row and
	// the value to fix.
	Denied bool `json:"denied,omitempty"`

	// Because explains every refusal in this verdict, in one sentence each. It
	// is what the SECURITY log line and the operator's answer both quote, so a
	// clamp is never something only a server log knows about.
	Because []string `json:"because,omitempty"`
}

// Refused reports whether this verdict takes anything away.
func (v CeilingVerdict) Refused() bool { return v.CapabilityRefused || v.ToolScopeRefused || v.Denied }

// CheckSpawn judges one spawn against the ceiling that governs its directory.
//
// TWO ARMS, and the second one is the reason the first is not theatre.
//
//	THE DECLARED ARM. `capability: frontier_plus` in a directory capped at
//	`frontier` is refused outright. This is the direct reading of the block and
//	the one a routed dispatch hits, because a dispatch acting on a routing
//	decision copies that decision's capability onto the spawn.
//
//	THE NAMED-MODEL ARM. A caller that simply does not declare a capability, or
//	declares a modest one while naming Fable, would walk around the first arm
//	entirely — the ceiling would govern a LABEL. So the model is looked up in the
//	matrix's own profiles and the capability it resolves to is judged too.
//
// THE NAMED-MODEL ARM READS AMBIGUITY AT ITS STRONGEST, and it used to read it
// at its weakest. A model with several readings in the matrix was judged by the
// LOWEST-ranked one, on the reasoning that a clamp firing on a legitimate
// reading is worse than one missing an illegitimate one. That is the right
// instinct for a scheduler and the wrong one for an authority gate: it hands a
// caller who omits `effort` the cheapest interpretation of the ask while the
// provider goes on to run the strongest one. `opus` with no effort is `frontier`
// (high), `deep_reviewer` (high) AND `frontier_max` (max); the question this arm
// answers is "could this be above the ceiling", and for `opus` under `frontier`
// the answer is yes.
//
// So: a model is refused when ANY (provider, model, effort) pairing in the
// matrix that could have produced it ranks above the ceiling, and a pairing the
// ranks block cannot rank counts as above every ceiling — an unjudgeable reading
// is not a safe one. Name `effort: high` and the reading narrows to what you
// actually meant; that is the disambiguation, and it costs one field.
//
// A model that appears nowhere in the matrix is still not judged at all: the
// matrix makes no claim about it, and inventing one would be a classification.
//
// UNRANKED FAILS CLOSED IN BOTH DIRECTIONS. A spawn naming a capability the file
// does not rank cannot be shown to be under the ceiling, so it is CLAMPED; a
// ceiling row naming one cannot judge anything, so the spawn is DENIED. The
// second half used to be the open direction — reported and skipped — which made
// a typo in the policy file the quietest possible way to delete the policy.
// validate() reports both cases at load; neither is left to the log alone.
func (m *Matrix) CheckSpawn(req SpawnRequest) CeilingVerdict {
	var v CeilingVerdict
	if m == nil {
		return v
	}
	ceiling, key := m.CeilingFor(req.CanonicalCwd)
	v.Key, v.MaxCapability, v.MaxToolScope = key, ceiling.MaxCapability, ceiling.MaxToolScope
	if key == "" {
		return v
	}

	m.checkToolScope(req, ceiling, &v)
	m.checkCapability(req, ceiling, &v)
	return v
}

func (m *Matrix) checkToolScope(req SpawnRequest, ceiling Ceiling, v *CeilingVerdict) {
	want, wantOK := ToolScopeRank(req.ToolScope)
	if !wantOK {
		return // nothing asked for, or a spelling the security model does not have
	}
	max, maxOK := ToolScopeRank(ceiling.MaxToolScope)
	if !maxOK {
		if strings.TrimSpace(ceiling.MaxToolScope) != "" {
			// FAIL CLOSED. The row exists, it names a tier, and the tier is not
			// one — so this directory HAS a policy and the policy is unreadable.
			// Waving the spawn through would make a typo the most effective way
			// to remove a ceiling, and it would look identical to working.
			v.Denied = true
			v.Because = append(v.Because, fmt.Sprintf(
				"ceilings.%s.max_tool_scope is %q, which is not an authority tier (view, triage, operator) — this spawn is REFUSED rather than admitted, because a ceiling that cannot be read is not a ceiling that does not exist. Fix the value in ~/.config/workspacer-hub/routing.yaml; there is no bus call that can",
				v.Key, ceiling.MaxToolScope))
		}
		return
	}
	if want <= max {
		return
	}
	v.ToolScopeRefused, v.ToolScope = true, strings.ToLower(strings.TrimSpace(ceiling.MaxToolScope))
	v.Because = append(v.Because, fmt.Sprintf(
		"this spawn asked for the %s tool tier in %s, and routing.yaml's ceilings.%s caps that directory at %s — the tier is clamped to %s. Raise it by editing ceilings: in ~/.config/workspacer-hub/routing.yaml; there is no bus call that can",
		strings.ToLower(strings.TrimSpace(req.ToolScope)), req.CanonicalCwd, v.Key, v.MaxToolScope, v.MaxToolScope))
}

func (m *Matrix) checkCapability(req SpawnRequest, ceiling Ceiling, v *CeilingVerdict) {
	if strings.TrimSpace(ceiling.MaxCapability) == "" {
		return
	}
	maxRank := m.RankOf(ceiling.MaxCapability)
	if maxRank == UnrankedCapability {
		// FAIL CLOSED, same reasoning as the tier arm above: the row says this
		// directory is capped and the cap is unrankable, so nothing here can be
		// compared. Reporting it and continuing turned one unranked word into a
		// silent removal of the whole capability ceiling for that tree.
		v.Denied = true
		v.Because = append(v.Because, fmt.Sprintf(
			"ceilings.%s.max_capability is %q, which routing.yaml's capability_ranks: block does not rank — this spawn is REFUSED rather than admitted, because an unrankable ceiling cannot be shown to permit anything. Rank it under capability_ranks: and the ceiling starts working",
			v.Key, ceiling.MaxCapability))
		return
	}

	declared := strings.ToLower(strings.TrimSpace(req.Capability))
	if declared != "" {
		rank := m.RankOf(declared)
		if rank == UnrankedCapability {
			v.CapabilityRefused, v.Capability = true, ceiling.MaxCapability
			v.Because = append(v.Because, fmt.Sprintf(
				"this spawn declared capability %q, which routing.yaml's capability_ranks: block does not rank — an unrankable capability cannot be shown to sit under ceilings.%s's %s, so it is clamped to %s rather than admitted on the benefit of the doubt",
				declared, v.Key, ceiling.MaxCapability, ceiling.MaxCapability))
			return
		}
		if rank > maxRank {
			v.CapabilityRefused, v.Capability = true, ceiling.MaxCapability
			v.Because = append(v.Because, fmt.Sprintf(
				"this spawn asked for capability %s in %s, and routing.yaml's ceilings.%s caps that directory at %s — the capability is clamped to %s and the model/effort it named are dropped, so the spawn cannot keep the model the refused capability chose",
				declared, req.CanonicalCwd, v.Key, ceiling.MaxCapability, ceiling.MaxCapability))
			return
		}
	}

	// The named-model arm. Only reached when the declared capability was absent
	// or already under the ceiling, which is exactly the shape the label-only
	// reading of this block would wave through.
	named, capName, ok := m.capabilityOfModel(req.Provider, req.Model, req.Effort)
	if !ok || named <= maxRank {
		return
	}
	v.CapabilityRefused, v.Capability = true, ceiling.MaxCapability
	v.Because = append(v.Because, fmt.Sprintf(
		"this spawn named %s %s%s, which routing.yaml can read as capability %s — above ceilings.%s's %s for %s. The STRONGEST reading of an ambiguous model is the one an authority gate has to judge, so naming `effort` narrows it to what you meant. Declaring a lower `capability` does not change what the model IS, which is why this arm exists",
		req.Provider, req.Model, effortSuffix(req.Effort), capName, v.Key, ceiling.MaxCapability, req.CanonicalCwd))
}

// capabilityOfModel answers what capability a named (provider, model, effort)
// could be in this matrix, AT ITS STRONGEST.
//
// It returns the HIGHEST rank among every pairing that could have produced the
// request. That is the reading LEAST favourable to the caller, and it is the
// only sound one for a gate: the caller does not choose which reading the
// provider runs, so "it might have meant the cheap one" is not a fact about what
// will execute. An effort the caller left unspecified matches every effort the
// matrix pairs with that model; an effort they DID specify must match exactly,
// which is how a caller narrows the reading to the one they meant.
//
// AN UNRANKED PAIRING OUTRANKS EVERYTHING. A reading the ranks block cannot rank
// cannot be shown to sit under any ceiling, so it is reported at
// UnrankedCapabilityStrength — above every real rank — rather than abandoning
// the judgement, which is what the old code did and what let one unranked
// profile entry disable the model arm for that model everywhere.
//
// `ok` is false only when the matrix has NO reading at all: an unknown model, or
// none named. A matrix that never mentions a model makes no claim about it.
func (m *Matrix) capabilityOfModel(provider, model, effort string) (rank int, capability string, ok bool) {
	p := normalizeProvider(provider)
	mo := strings.ToLower(strings.TrimSpace(model))
	ef := strings.ToLower(strings.TrimSpace(effort))
	if p == "" || mo == "" {
		return 0, "", false
	}
	best, bestName, found := 0, "", false
	for _, pname := range sortedKeys(m.Profiles) {
		for _, cname := range sortedKeys(m.Profiles[pname]) {
			a := m.Profiles[pname][cname]
			if normalizeProvider(a.Provider) != p || strings.ToLower(strings.TrimSpace(a.Model)) != mo {
				continue
			}
			if ef != "" && strings.ToLower(strings.TrimSpace(a.Effort)) != ef {
				continue
			}
			r := m.RankOf(cname)
			if r == UnrankedCapability {
				r = UnrankedCapabilityStrength
			}
			if !found || r > best {
				best, bestName, found = r, cname, true
			}
		}
	}
	return best, bestName, found
}
