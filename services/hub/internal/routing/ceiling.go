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

	// Because explains every refusal in this verdict, in one sentence each. It
	// is what the SECURITY log line and the operator's answer both quote, so a
	// clamp is never something only a server log knows about.
	Because []string `json:"because,omitempty"`
}

// Refused reports whether this verdict takes anything away.
func (v CeilingVerdict) Refused() bool { return v.CapabilityRefused || v.ToolScopeRefused }

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
// The named-model arm is deliberately UNAMBIGUOUS-ONLY. A model is judged above
// the ceiling only when EVERY (provider, model, effort) pairing in the whole
// matrix that could have produced it ranks above the ceiling. `opus` with no
// effort named appears as `frontier` (high), `deep_reviewer` (high) AND
// `frontier_max` (max), so it is not refused — the caller may well have meant
// one of the first two, and a clamp that fires on a legitimate reading is worse
// than one that misses an illegitimate one, because it breaks working dispatches
// and teaches people to raise the ceiling. `opus` WITH `effort: max` matches only
// frontier_max and is refused; `fable` is only ever frontier_plus and is refused.
// A model that appears nowhere in the matrix is not judged at all: the matrix
// makes no claim about it, and inventing one would be a classification.
//
// UNRANKED FAILS CLOSED on the requested side and OPEN on the ceiling's side.
// A spawn naming a capability the file does not rank cannot be shown to be under
// the ceiling, so it is clamped; a CEILING naming an unranked capability cannot
// judge anything, so it judges nothing and says so. Those are the two honest
// directions, and validate() reports both cases at load.
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
			v.Because = append(v.Because, fmt.Sprintf(
				"ceilings.%s.max_tool_scope is %q, which is not an authority tier (view, triage, operator), so no scope ceiling could be applied here",
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
		v.Because = append(v.Because, fmt.Sprintf(
			"ceilings.%s.max_capability is %q, which routing.yaml's capability_ranks: block does not rank, so no capability ceiling could be applied here — rank it and the ceiling starts working",
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
		"this spawn named %s %s%s, which every profile in routing.yaml resolves at capability %s, above ceilings.%s's %s for %s — the model and effort are dropped so the provider falls back to its own configured default. Declaring a lower `capability` does not change what the model IS, which is why this arm exists",
		req.Provider, req.Model, effortSuffix(req.Effort), capName, v.Key, ceiling.MaxCapability, req.CanonicalCwd))
}

// capabilityOfModel answers, unambiguously or not at all, what capability a
// named (provider, model, effort) is in this matrix.
//
// It returns the LOWEST rank among every pairing that could have produced the
// request, because that is the reading most favourable to the caller and the
// clamp is only entitled to fire when even that reading is above the ceiling.
// An effort the caller left unspecified matches every effort the matrix pairs
// with that model; an effort they DID specify must match exactly.
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
				// An unranked pairing is a reading we cannot judge, and an
				// unjudgeable reading is a reading the caller might have meant.
				// Refuse to conclude anything from the model at all.
				return 0, "", false
			}
			if !found || r < best {
				best, bestName, found = r, cname, true
			}
		}
	}
	return best, bestName, found
}
