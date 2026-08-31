package routing

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// shippedMatrix is the compiled-in default with the fallback wired, which is
// what a running hub holds when there is no file.
func shippedMatrix(t *testing.T) *Matrix {
	t.Helper()
	m, err := Load("", nil)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	return m
}

// TestTheShippedMatrixNeverRefusesItsOwnAnswer is the coherence test, and it is
// the one that changed a shipped default.
//
// A ceiling that refuses the policy it sits under is not a ceiling, it is a
// disagreement: routing.select would answer "Fable for the judge", the spawn
// gate would take the model away, and the dispatch would arrive as an
// unexplained downgrade every single time. So EVERY role this matrix answers,
// in EVERY profile, and under EVERY mode shift, must pass the matrix's own
// `default` ceiling. Capping is a per-directory act — see the sibling tests,
// which configure one and watch it bind.
//
// It also covers the case that made the capability ladder necessary at all:
// `capabilities:` lists reviewer and deep_reviewer AFTER frontier while they
// resolve to Sonnet High and Opus High, so reading list position as strength
// would refuse a reviewer for being "above" a model that costs more than it.
func TestTheShippedMatrixNeverRefusesItsOwnAnswer(t *testing.T) {
	m := shippedMatrix(t)

	check := func(what, profile, capability string) {
		t.Helper()
		a, err := m.ResolveCapability(profile, capability)
		if err != nil {
			t.Fatalf("%s: %v", what, err)
		}
		v := m.CheckSpawn(SpawnRequest{
			CanonicalCwd: "/home/someone/project",
			Capability:   capability,
			Provider:     a.Provider,
			Model:        a.Model,
			Effort:       a.Effort,
		})
		if v.CapabilityRefused {
			t.Errorf("%s (capability %s, %s %s) is REFUSED by this matrix's own default ceiling: %s",
				what, capability, a.Provider, a.Model, strings.Join(v.Because, " | "))
		}
	}

	for profile := range m.Profiles {
		for role, capability := range m.Roles {
			check("role "+role+" under profile "+profile, profile, capability)
		}
		for mode, byRole := range m.ModeShifts {
			for role, capability := range byRole {
				check("role "+role+" shifted by "+mode+" under profile "+profile, profile, capability)
			}
		}
	}
}

func TestCheckSpawnClampsTheDeclaredCapability(t *testing.T) {
	// A configured ceiling, because the SHIPPED default deliberately admits
	// everything this matrix answers (see the coherence test above).
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus"})
	if !v.CapabilityRefused || v.Capability != "frontier" {
		t.Fatalf("frontier_plus was not clamped to frontier: %+v", v)
	}
	if v.Key != CeilingDefaultKey {
		t.Errorf("matched ceiling key %q, want %q", v.Key, CeilingDefaultKey)
	}
	if len(v.Because) == 0 {
		t.Error("a refusal with no sentence — the caller and the log both quote Because")
	}
	// At or below is untouched.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier"}); v.CapabilityRefused {
		t.Errorf("frontier refused under a frontier ceiling: %+v", v)
	}
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "reviewer"}); v.CapabilityRefused {
		t.Errorf("reviewer (Sonnet High, rank 2) refused under a frontier ceiling: %+v", v)
	}
}

// The named-model arm: a caller that declares a modest capability while naming a
// reserved model is naming the escalation, whatever the label says.
func TestCheckSpawnJudgesTheModelAndNotOnlyTheLabel(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}

	// fable is only ever frontier_plus in this matrix — unambiguous, refused.
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "balanced", Provider: "claude", Model: "fable"})
	if !v.CapabilityRefused {
		t.Errorf("a spawn declaring `balanced` while naming fable was admitted — the ceiling is governing a label: %+v", v)
	}

	// sol at xhigh is frontier_max and nothing else — refused.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "codex", Model: "gpt-5.6-sol", Effort: "xhigh"}); !v.CapabilityRefused {
		t.Errorf("codex sol xhigh (frontier_max) was admitted under a frontier ceiling: %+v", v)
	}

	// AMBIGUITY IS READ AT ITS STRONGEST, and this assertion is the REVERSE of
	// what it used to be. `opus` with no effort is frontier (anthropic_only,
	// high), deep_reviewer (mixed, high) AND frontier_max (anthropic_only, max).
	// The old rule took the cheapest reading and admitted it; that hands a caller
	// who simply omits `effort` the benefit of an interpretation the provider is
	// under no obligation to run. A gate judges what COULD execute.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus"}); !v.CapabilityRefused {
		t.Errorf("claude opus with no effort was admitted under a frontier ceiling, but it reads as frontier_max too — omitting a field must not buy the cheapest interpretation: %+v", v)
	}
	// Naming the effort narrows the reading, and a narrowed reading UNDER the
	// ceiling is admitted. This is what keeps the strongest-reading rule from
	// being an over-refusal with no remedy: the remedy is one field.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus", Effort: "high"}); v.CapabilityRefused {
		t.Errorf("claude opus at effort high is frontier/deep_reviewer, both at or under a frontier ceiling; it was refused: %+v", v)
	}
	// …and the effort that makes it unambiguously frontier_max still refuses.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus", Effort: "max"}); !v.CapabilityRefused {
		t.Errorf("claude opus at effort max is frontier_max and only that; it was admitted: %+v", v)
	}
	// A model this matrix never mentions is not judged.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "some-unknown-model"}); v.CapabilityRefused {
		t.Errorf("a model the matrix says nothing about was refused — the clamp invented a classification: %+v", v)
	}
}

// An unranked capability cannot be shown to be under the ceiling, so it is
// clamped rather than admitted on the benefit of the doubt — and the load
// reports the gap so it is found when the file is saved.
func TestUnrankedCapabilityFailsClosedAndIsReportedAtLoad(t *testing.T) {
	m, err := Load("test.yaml", []byte("capabilities:\n  - cheap\n  - balanced\n  - frontier\n  - frontier_max\n  - reviewer\n  - deep_reviewer\n  - frontier_plus\n  - wildcard\nceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "wildcard"})
	if !v.CapabilityRefused {
		t.Errorf("an unranked capability was admitted under a ceiling that cannot compare it: %+v", v)
	}
	found := false
	for _, iss := range m.Issues {
		if strings.Contains(iss.Where, "capability_ranks") && strings.Contains(iss.Detail, "wildcard") {
			found = true
		}
	}
	if !found {
		t.Errorf("the load did not report the unranked capability, so an operator meets it as a refused spawn instead: %v", m.Issues)
	}
}

// A CEILING naming an unranked capability judges nothing, and says so rather
// than clamping everything.
func TestUnrankedCeilingJudgesNothing(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: mystery, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus"})
	if v.CapabilityRefused {
		t.Errorf("a ceiling that cannot rank its own limit refused a spawn anyway: %+v", v)
	}
	if len(v.Because) == 0 || !strings.Contains(strings.Join(v.Because, " "), "capability_ranks") {
		t.Errorf("the unusable ceiling said nothing about why: %+v", v)
	}
}

func TestCheckSpawnClampsTheToolScope(t *testing.T) {
	m, err := Load("test.yaml", []byte(
		"ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n  /home/me/client: { max_capability: balanced, max_tool_scope: triage }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/home/me/client/sub", ToolScope: "operator", Capability: "frontier"})
	if !v.ToolScopeRefused || v.ToolScope != "triage" {
		t.Errorf("operator was not clamped to triage inside a triage-capped tree: %+v", v)
	}
	if !v.CapabilityRefused || v.Capability != "balanced" {
		t.Errorf("frontier was not clamped to balanced inside a balanced-capped tree: %+v", v)
	}
	if v.Key != "/home/me/client" {
		t.Errorf("matched ceiling %q, want the ancestor entry", v.Key)
	}
	// A sibling whose name shares the prefix is NOT inside it.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/home/me/client-old", ToolScope: "operator"}); v.ToolScopeRefused {
		t.Errorf("/home/me/client-old was treated as inside /home/me/client: %+v", v)
	}
}

// The ceiling is looked up on a path that is ALREADY canonical, and this test
// exists to keep that precondition visible: hand CheckSpawn the unresolved
// spelling and the capped directory is missed entirely. The enforcement site is
// what resolves; nothing here does, and nothing here should.
func TestCeilingLookupNeedsAnAlreadyCanonicalPath(t *testing.T) {
	dir := t.TempDir()
	real := filepath.Join(dir, "client")
	if err := os.MkdirAll(real, 0o755); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(dir, "shortcut")
	if err := os.Symlink(real, link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	m, err := Load("test.yaml", []byte(
		"ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n  "+real+": { max_capability: cheap, max_tool_scope: view }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: real, ToolScope: "operator"}); !v.ToolScopeRefused {
		t.Fatalf("the capped directory did not clamp on its own canonical path: %+v", v)
	}
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: link, ToolScope: "operator"}); v.ToolScopeRefused {
		t.Fatalf("the symlink spelling matched the capped entry — CeilingFor is lexical and must not appear to resolve links, or the enforcement site's canonicalization stops being load-bearing: %+v", v)
	}
}

// AN AUTHORITY GATE FAILS CLOSED IN BOTH DIRECTIONS. These two pin the half that
// used to fail OPEN: a ceiling row the file cannot parse. It was reported and
// then skipped, so a typo in the policy silently deleted the policy for that
// tree — and the result was indistinguishable from a directory nobody had
// capped.

func TestAnUnrankableConfiguredCeilingDeniesRatherThanAdmits(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontierr, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus", Model: "fable", Provider: "claude"})
	if !v.Denied {
		t.Fatalf("a ceiling whose max_capability cannot be ranked admitted a frontier_plus spawn: %+v", v)
	}
	if !v.Refused() {
		t.Error("Denied must count as a refusal — the audit and the caller both read Refused()")
	}
	if len(v.Because) == 0 || !strings.Contains(v.Because[0], "frontierr") {
		t.Errorf("the refusal must quote the unrankable value so the operator knows what to fix: %v", v.Because)
	}
}

func TestAnUnknownConfiguredToolTierDeniesRatherThanAdmits(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier, max_tool_scope: opperator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", ToolScope: "operator"})
	if !v.Denied {
		t.Fatalf("a ceiling whose max_tool_scope is not a tier admitted an operator-tier spawn: %+v", v)
	}
	// A spawn asking for NO tier is not judged by the tier arm at all, so an
	// unparseable tier row must not deny it: the row it cannot read is one this
	// spawn was never going to be measured against.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "balanced"}); v.Denied {
		t.Errorf("a spawn asking for no tool tier was denied by an unreadable TIER row: %+v", v)
	}
}

// An empty max_tool_scope is ABSENCE, not a typo: the row simply does not cap the
// authority axis, and a deny there would refuse every spawn under a
// capability-only ceiling.
func TestACeilingRowThatOmitsATierDoesNotDeny(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", ToolScope: "operator", Capability: "balanced"}); v.Denied || v.ToolScopeRefused {
		t.Errorf("an omitted max_tool_scope was treated as a broken one: %+v", v)
	}
}

// An unranked PAIRING in the profiles now outranks every ceiling instead of
// abandoning the model arm. Before, one unrankable profile entry naming a model
// turned the named-model check off for that model everywhere in the matrix.
func TestAnUnrankablePairingMakesTheModelArmRefuseRatherThanGiveUp(t *testing.T) {
	m, err := Load("test.yaml", []byte(
		"capabilities:\n  - cheap\n  - balanced\n  - frontier\n  - frontier_max\n  - reviewer\n  - deep_reviewer\n  - frontier_plus\n  - wildcard\n"+
			"profiles:\n  mixed:\n    wildcard: { provider: codex, model: gpt-5.6-luna }\n"+
			"ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	// gpt-5.6-luna is `cheap` (rank 1) AND now also the unranked `wildcard`.
	// The unrankable reading is the one that decides.
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "codex", Model: "gpt-5.6-luna"})
	if !v.CapabilityRefused {
		t.Errorf("a model with an UNRANKABLE reading was admitted under a frontier ceiling: %+v", v)
	}
}

// THE CLAMP MUST LEAVE NO HOLE. Deleting the refused model hands the choice to
// the provider's own configured default — `opus[1m]` on the desktop's Claude
// path, a model this matrix never mentions and therefore never judges. These pin
// the replacement tuple that closes it.

func TestARefusedCapabilityNamesTheModelThePermittedOneResolvesTo(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{
		CanonicalCwd: "/x", Capability: "frontier_plus", Provider: "claude", Model: "fable",
	})
	if !v.CapabilityRefused {
		t.Fatalf("frontier_plus was not refused under a frontier ceiling: %+v", v)
	}
	if v.Model == "" {
		t.Fatal("the verdict named no replacement model — the provider's own default would fill the hole, which is the relabelling this exists to stop")
	}
	if v.Model == "fable" {
		t.Errorf("the refused model survived as the replacement: %+v", v)
	}
	if v.Provider != "claude" {
		t.Errorf("the clamp moved a claude spawn onto %q — a ceiling may lower authority, not swap the harness: %+v", v.Provider, v)
	}
	// And the replacement must itself be at or under the ceiling, or the clamp
	// has merely picked a different way to exceed it.
	rank, _, ok := m.capabilityOfModel(v.Provider, v.Model, v.Effort)
	if !ok {
		t.Fatalf("the replacement %s %s is not a pairing this matrix knows — it cannot be shown to be under the ceiling", v.Provider, v.Model)
	}
	if rank > m.RankOf("frontier") {
		t.Errorf("the replacement %s %s%s reads above the frontier ceiling it was chosen to satisfy", v.Provider, v.Model, effortSuffix(v.Effort))
	}
}

// A spawn that declares a too-high capability and names NO model is the shape
// that made deletion useless: there was nothing to delete, and the provider
// default fired anyway.
func TestARefusedCapabilityWithNoModelNamedStillGetsATuple(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: balanced, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus"})
	if !v.CapabilityRefused {
		t.Fatalf("frontier_plus was not refused under a balanced ceiling: %+v", v)
	}
	if v.Model == "" || v.Provider == "" {
		t.Fatalf("no tuple for a spawn that named no model — the provider default is exactly what fills that gap: %+v", v)
	}
}

// The model names its provider even when the spawn does not. `model: fable`
// plainly means claude, and answering it with a codex model because the active
// profile prefers codex would be a re-route nobody asked for.
func TestTheReplacementFollowsTheModelsOwnProviderWhenNoneWasNamed(t *testing.T) {
	m, err := Load("test.yaml", []byte("active_profile: mixed\nceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus", Model: "fable"})
	if v.Provider != "claude" {
		t.Errorf("a spawn naming fable (a claude model) and no provider was routed to %q — mixed's frontier is codex, but the model said claude: %+v", v.Provider, v)
	}
}

// A provider the matrix holds no profile entry for gets NO tuple and NO
// substitution: there was never a matrix opinion about its models for the
// ceiling to protect, and silently moving the spawn onto codex would be a far
// worse surprise than the drop.
func TestAProviderTheMatrixDoesNotServeIsNotReRouted(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: balanced, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus", Provider: "copilot"})
	if !v.CapabilityRefused {
		t.Fatalf("the declared capability was not clamped: %+v", v)
	}
	if v.Provider != "" || v.Model != "" {
		t.Errorf("a copilot spawn was re-routed onto %s %s: %+v", v.Provider, v.Model, v)
	}
	joined := strings.Join(v.Because, " | ")
	if !strings.Contains(joined, "copilot") {
		t.Errorf("the verdict did not say why no replacement was named: %v", v.Because)
	}
}
