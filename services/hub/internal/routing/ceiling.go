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
	// Role is the spawn's declared `role` param — one of the names under
	// `roles:`. It is read by the FRESHNESS arm only (see fresh.go), never by
	// the ceiling: a ceiling that a caller could raise by relabelling its role
	// would be a ceiling made of a caller-supplied string.
	Role string
	// Resuming says the spawn carries a `resumeSessionId`, so it would CONTINUE
	// an existing conversation rather than start one. ResumeSessionID is the id
	// itself, quoted back in the refusal so the caller can see which session it
	// asked to inherit.
	Resuming        bool
	ResumeSessionID string
	// ToolScope is the AUTHORITY tier the spawn asks for the child to hold, or
	// "operator" spelled as the legacy `mcpFacade: true`.
	ToolScope string
	// Provider / Model / Effort are what the spawn actually names. They exist
	// here for the arm that catches a spawn declaring a modest capability while
	// naming a reserved model — see CheckSpawn.
	Provider string
	Model    string
	Effort   string

	// SkipReplacementRouting suppresses the safe routed tuple a refusal
	// otherwise carries. It exists for ONE caller: Select, which consults the
	// ceiling before resolving a capability to a model and then does that
	// resolution itself, through the profile and provider logic the uncapped
	// answer used. Letting the verdict also name a model there would put a
	// second, differently-derived tuple in the explanation of one decision.
	//
	// FALSE IS THE SAFE DEFAULT, deliberately: the SPAWN GATE must never forget
	// to ask for the replacement, because for it "no replacement" means an
	// omitted model and an omitted model is the provider's own default.
	SkipReplacementRouting bool
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

	// Provider / Model / Effort are the SAFE ROUTED TUPLE for the clamped
	// capability, and they are why the capability clamp is a limit rather than
	// a relabelling.
	//
	// Deleting the refused model was not enough. Every provider resolves an
	// OMITTED model to its own configured default, and the desktop's Claude
	// default is `opus[1m]`. So a ceiling that refused `frontier_plus`/`fable`
	// handed the spawn back to a default that is plausibly just as strong, one
	// layer below where the ceiling could see it. The fix is to leave no hole:
	// the clamp names what the permitted capability actually resolves to, and
	// the enforcement site WRITES it.
	//
	// (`opus[1m]` used to walk past the named-model arm as well, because the
	// matrix spells that model `opus` and the two strings are not equal. The
	// arm now compares on the model WITHOUT its context-window suffix, so the
	// default is judged as the `opus` it is. See modelid.go.)
	//
	// Empty when the matrix cannot route the permitted capability on the
	// provider this spawn is for — a provider it holds no profile entry for
	// (copilot, opencode, pi) has no matrix opinion about its models at all, so
	// there is nothing here for the ceiling to have been protecting. The
	// enforcement site falls back to deleting, and Because says so.
	Provider string `json:"provider,omitempty"`
	Model    string `json:"model,omitempty"`
	Effort   string `json:"effort,omitempty"`

	// ResumeRefused says this spawn declared work the matrix marks `fresh: true`
	// and asked to RESUME a session, which the two cannot both be. Like Denied it
	// stops the spawn rather than clamping it, and for a reason a clamp cannot
	// express: dropping `resumeSessionId` would hand back a NEW session that the
	// caller believes is a continuation, which is a worse failure than a refusal.
	// FreshCapability names the entry whose flag refused it. See fresh.go.
	ResumeRefused   bool   `json:"resumeRefused,omitempty"`
	FreshCapability string `json:"freshCapability,omitempty"`

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
func (v CeilingVerdict) Refused() bool {
	return v.CapabilityRefused || v.ToolScopeRefused || v.Denied || v.ResumeRefused
}

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
// A model that appears there under a different CONTEXT WINDOW does appear:
// `opus[1m]` is judged as `opus`, because the suffix asks for a bigger window on
// the same model rather than naming another one. That was the last hole in this
// arm, and it was the one an omitted `model` fell through by default.
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
	// THE FRESHNESS ARM IS NOT A CEILING ARM, and it runs FIRST so that it runs
	// at all: the ceiling lookup below returns early for a matrix with no
	// `ceilings:` block, and a reviewer's independence does not depend on
	// whether anyone capped the directory it works in. See fresh.go.
	m.checkFresh(req, &v)

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
				"this spawn asked for capability %s in %s, and routing.yaml's ceilings.%s caps that directory at %s — the capability is clamped to %s and the model/effort it named are replaced with what %s actually resolves to, so the spawn cannot keep the model the refused capability chose",
				declared, req.CanonicalCwd, v.Key, ceiling.MaxCapability, ceiling.MaxCapability, ceiling.MaxCapability))
			m.routeSafely(req, ceiling.MaxCapability, v)
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
	m.routeSafely(req, ceiling.MaxCapability, v)
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
// THE CONTEXT-WINDOW SUFFIX IS NOT PART OF THE MODEL. Both sides of the
// comparison are matched with `[1m]` / `-1m` taken off, so `opus[1m]` is read as
// the `opus` the matrix has entries for. It is a request for a bigger window on
// the same model, and before this the mismatch meant the desktop's own shipped
// default was the one Claude model no ceiling could judge. Nothing is rewritten:
// see modelid.go for why the suffix survives to the spawn.
//
// `ok` is false only when the matrix has NO reading at all: an unknown model, or
// none named. A matrix that never mentions a model makes no claim about it.
func (m *Matrix) capabilityOfModel(provider, model, effort string) (rank int, capability string, ok bool) {
	p := normalizeProvider(provider)
	mo := matchableModel(model)
	ef := strings.ToLower(strings.TrimSpace(effort))
	if p == "" || mo == "" {
		return 0, "", false
	}
	best, bestName, found := 0, "", false
	for _, pname := range sortedKeys(m.Profiles) {
		for _, cname := range sortedKeys(m.Profiles[pname]) {
			a := m.Profiles[pname][cname]
			if normalizeProvider(a.Provider) != p || matchableModel(a.Model) != mo {
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

// routeSafely fills in the verdict's safe routed tuple: what the PERMITTED
// capability actually resolves to for the provider this spawn is for.
//
// WITHOUT THIS, THE CLAMP IS A RELABELLING. The enforcement site used to delete
// `model` and `effort` when it lowered `capability`, on the reasoning that
// keeping the model a refused capability chose would change a label rather than
// apply a limit. True as far as it goes, and then the provider resolved the
// now-omitted model to its OWN configured default, one layer below where any
// ceiling can see it. A ceiling that turns "an explicit strong model" into "an
// omitted model that defaults to a strong model" has relabelled.
//
// THE REPLACEMENT CARRIES THE MATRIX ENTRY'S OWN WINDOW, not the refused
// request's. A spawn asking for `opus[1m]` and clamped to `balanced` is answered
// with what routing.yaml spells for balanced, verbatim, so the substituted model
// gets the context window its own entry implies. Grafting the refused `[1m]`
// onto it would invent an id the matrix never named and the load-time catalog
// check never validated, and it would be nonsense the moment the replacement
// lands on a provider with no such vocabulary. Write the suffix on the profile
// entry if a capped directory should still run 1M. The drop is stated in
// Because rather than left silent, on the same no-silent-downgrades rule the
// rest of this verdict follows.
//
// WHICH PROVIDER, and why the answer is not simply "the one the active profile
// prefers". Substituting the provider is a bigger change than substituting the
// model: it swaps the harness the work runs in. So the constraint is, in order:
//
//  1. the provider the spawn NAMED, if it named one;
//  2. the provider the matrix associates with the MODEL it named, when that is
//     unambiguous — `model: fable` with no provider plainly means claude, and
//     answering it with a codex model because the active profile prefers codex
//     would be a re-route nobody asked for;
//  3. otherwise the active profile's own answer, unconstrained.
//
// A constrained resolution that finds nothing leaves the tuple EMPTY rather than
// substituting a provider. That is the copilot/opencode/pi case: the matrix
// holds no profile entry for them, so it has no opinion about their models, and
// there was never anything on that axis for the ceiling to protect. Silently
// moving such a spawn onto codex would be a far worse surprise than the delete.
func (m *Matrix) routeSafely(req SpawnRequest, capability string, v *CeilingVerdict) {
	if m == nil || req.SkipReplacementRouting || strings.TrimSpace(capability) == "" {
		return
	}
	profile, _ := m.ActiveProfileName()

	constraint := normalizeProvider(req.Provider)
	if constraint == "" {
		constraint = m.providerOfModel(req.Model)
	}

	a, from, ok := m.assignmentFor(profile, capability, constraint, constraint != "")
	if ok && !a.IsEnabled() {
		ok = false // `enabled: false` is not something to route a clamp onto
	}
	if !ok {
		if constraint != "" {
			v.Because = append(v.Because, fmt.Sprintf(
				"routing.yaml puts no spawnable %s pairing on provider %s, so the model and effort are dropped rather than moved to another provider — the matrix expresses no opinion about %s's models, and re-routing a spawn onto a different harness is a bigger surprise than a ceiling is entitled to. The provider then starts on its own configured default, which this ceiling cannot see",
				capability, constraint, constraint))
			return
		}
		v.Because = append(v.Because, fmt.Sprintf(
			"no profile in this matrix resolves %s to anything spawnable, so the model and effort are dropped and the provider starts on its own configured default — which this ceiling cannot see. Give %s an entry under profiles: to close that",
			capability, capability))
		return
	}

	v.Provider, v.Model, v.Effort = a.Provider, a.Model, a.Effort
	v.Because = append(v.Because, fmt.Sprintf(
		"the spawn is routed to %s %s%s instead, which is what profile %s resolves %s to — the ceiling NAMES the replacement rather than dropping the model, because an omitted model resolves to the provider's own configured default and that default is not something this ceiling can see",
		v.Provider, v.Model, effortSuffix(v.Effort), from, capability))
	noteWindowNotCarried(req.Model, v)
}

// noteWindowNotCarried says out loud that a context-window request did not
// survive a substitution.
//
// A refused `opus[1m]` is answered with whatever routing.yaml spells for the
// permitted capability, verbatim, so the replacement runs the window ITS OWN
// entry implies. That is the right rule (see routeSafely), and it is still a
// second thing the caller asked for and did not get, on an axis nothing else in
// the verdict mentions: `escalationScrubbed` records that `model` was taken, not
// that a 1M window went with it. So it gets a sentence, for the same reason
// every other refusal here does.
func noteWindowNotCarried(requested string, v *CeilingVerdict) {
	_, want := splitModelWindowSuffix(requested)
	if want == "" {
		return
	}
	if _, got := splitModelWindowSuffix(v.Model); got != "" {
		return // the replacement asks for a window of its own
	}
	v.Because = append(v.Because, fmt.Sprintf(
		"the %s context-window request on %q does not carry over: a substituted model runs the window its own routing.yaml entry implies, not the one the REFUSED model asked for, because pasting %s onto %q would invent an id the matrix never names and the load-time catalog check never validated. Spell the suffix on the profiles: entry if a capped directory should still run that window",
		want, strings.TrimSpace(requested), want, v.Model))
}

// providerOfModel is the provider a model plainly belongs to in this matrix, or
// "" when the matrix never mentions it or pairs it with more than one provider.
//
// It exists for exactly one job: keeping a clamp on a spawn that named a model
// but no provider from silently changing which harness runs. `model: fable`
// means claude whether or not the caller said so, and `model: opus[1m]` means
// claude for the same reason it means `opus`: the comparison ignores the
// context-window suffix on both sides.
func (m *Matrix) providerOfModel(model string) string {
	mo := matchableModel(model)
	if m == nil || mo == "" {
		return ""
	}
	found := ""
	for _, pname := range sortedKeys(m.Profiles) {
		for _, cname := range sortedKeys(m.Profiles[pname]) {
			a := m.Profiles[pname][cname]
			if matchableModel(a.Model) != mo {
				continue
			}
			p := normalizeProvider(a.Provider)
			if p == "" {
				continue
			}
			if found != "" && found != p {
				return "" // two providers serve this id: no plain reading
			}
			found = p
		}
	}
	return found
}
