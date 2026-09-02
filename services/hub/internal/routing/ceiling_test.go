package routing

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
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

// TestTheShippedMatrixNeverAdvisesWhatItsOwnCeilingRefuses is the coherence
// test, and it is the one that decides what the shipped default may be.
//
// A ceiling that refuses the policy it sits under is not a ceiling, it is a
// disagreement: routing.select answers "Fable for the judge", the spawn gate
// takes the model away, and the dispatch arrives as an unexplained downgrade
// every single time.
//
// IT USED TO BE STATED ONE LEVEL LOWER, and that is why the shipped default had
// to be permissive. The old assertion was "CheckSpawn never refuses
// ResolveCapability's answer" — a claim about the raw profile table, which
// forced `default: frontier_plus` because `roles.judge` is frontier_plus. The
// claim that actually matters is about the ADVICE: Select consults the ceiling
// before it resolves a capability to a model, so it caps its own answer and the
// gate has nothing left to disagree with. Restated at that level, a protective
// default and a self-consistent matrix stop being in tension — and the test is
// strictly stronger, because it exercises the tuple a dispatch will really
// carry, mode shifts and provider constraints included.
func TestTheShippedMatrixNeverAdvisesWhatItsOwnCeilingRefuses(t *testing.T) {
	m := shippedMatrix(t)
	const dir = "/home/someone/project"

	check := func(what, profile, role string, mode Mode) {
		t.Helper()
		d := Select(m, limits.Snapshot{}, nil, time.Unix(1_800_000_000, 0), Request{
			Role: role, Profile: profile, CanonicalCwd: dir,
		})
		if !d.Eligible {
			t.Errorf("%s: routing.select answered NOTHING SPAWNABLE under the shipped default ceiling: %s",
				what, strings.Join(d.Reason, " | "))
			return
		}
		v := m.CheckSpawn(SpawnRequest{
			CanonicalCwd: dir,
			Capability:   d.Capability,
			Provider:     d.Provider,
			Model:        d.Model,
			Effort:       d.Effort,
		})
		if v.Refused() {
			t.Errorf("%s: routing.select advised capability %s / %s %s%s and this matrix's OWN default ceiling refuses it: %s",
				what, d.Capability, d.Provider, d.Model, effortSuffix(d.Effort), strings.Join(v.Because, " | "))
		}
	}

	for profile := range m.Profiles {
		for role := range m.Roles {
			check("role "+role+" under profile "+profile, profile, role, ModeNormal)
		}
		for mode, byRole := range m.ModeShifts {
			for role := range byRole {
				check("role "+role+" under profile "+profile+" (mode_shifts."+mode+")", profile, role, Mode(mode))
			}
		}
	}
}

// THE JUDGE, NAMED. `roles.judge` is frontier_plus (rank 5) and the shipped
// default caps at frontier (rank 3), so this is the collision the coherence test
// above generalizes — and the one behaviour a user meets first. Pinned by name
// so a change to it is a decision somebody made rather than a number that
// drifted.
func TestWhatAJudgeGetsUnderTheShippedDefaultCeiling(t *testing.T) {
	m := shippedMatrix(t)
	d := Select(m, limits.Snapshot{}, nil, time.Unix(1_800_000_000, 0), Request{
		Role: "judge", CanonicalCwd: "/home/someone/project",
	})
	if !d.Eligible {
		t.Fatalf("a judge dispatch resolved to nothing: %s", strings.Join(d.Reason, " | "))
	}
	if d.BaseCapability != "frontier_plus" {
		t.Errorf("the role table no longer puts judge at frontier_plus (%q) — this test is describing something else now", d.BaseCapability)
	}
	if d.Capability != "frontier" {
		t.Errorf("a judge under the shipped default ceiling resolved to capability %q, want frontier", d.Capability)
	}
	// mixed is the shipped active profile, and its frontier is codex Sol High.
	if d.Provider != "codex" || d.Model != "gpt-5.6-sol" || d.Effort != "high" {
		t.Errorf("a judge under the shipped default got %s %s%s; the shipped `mixed` profile resolves frontier to codex gpt-5.6-sol high",
			d.Provider, d.Model, effortSuffix(d.Effort))
	}
	if d.Ceiling == nil || !d.Ceiling.CapabilityRefused {
		t.Fatalf("the decision does not report that a ceiling lowered it: %+v", d.Ceiling)
	}
	// NOT SILENT. The whole complaint about the old arrangement was an
	// unexplained downgrade; the explanation has to be in the answer.
	if !strings.Contains(strings.Join(d.Reason, " | "), "ceilings.default") {
		t.Errorf("the downgrade is not explained in the decision's own reason list: %v", d.Reason)
	}
}

// A directory with its OWN ceiling caps harder than the default, and Select
// honours it — the per-directory act the default is only a floor for.
func TestSelectHonoursAPerDirectoryCeiling(t *testing.T) {
	locked := filepath.Join(t.TempDir(), "locked")
	m, err := Load("test.yaml", []byte("ceilings:\n  "+strconv.Quote(locked)+": { max_capability: cheap, max_tool_scope: view }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	d := Select(m, limits.Snapshot{}, nil, time.Unix(1_800_000_000, 0), Request{
		Role: "implementer", CanonicalCwd: filepath.Join(locked, "sub", "dir"),
	})
	if !d.Eligible {
		t.Fatalf("nothing spawnable under a cheap ceiling: %s", strings.Join(d.Reason, " | "))
	}
	if d.Capability != "cheap" {
		t.Errorf("an implementer inside a directory capped at cheap resolved to %q", d.Capability)
	}
	if d.BaseCapability != "frontier" {
		t.Errorf("the base capability should still record what the ROLE asked for; got %q", d.BaseCapability)
	}
}

// A ceiling row nobody can parse makes Select REFUSE rather than advise. The
// gate would deny that spawn, so advising it would be the advise-then-refuse
// contradiction pointing the other way.
func TestSelectRefusesUnderAnUnreadableCeiling(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontierr, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	d := Select(m, limits.Snapshot{}, nil, time.Unix(1_800_000_000, 0), Request{
		Role: "implementer", CanonicalCwd: "/x",
	})
	if d.Eligible {
		t.Fatalf("routing advised %s %s under a ceiling it cannot read", d.Provider, d.Model)
	}
	if !strings.Contains(strings.Join(d.Reason, " | "), "frontierr") {
		t.Errorf("the refusal does not name the value to fix: %v", d.Reason)
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
	client := filepath.Join(t.TempDir(), "client")
	m, err := Load("test.yaml", []byte(
		"ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n  "+strconv.Quote(client)+": { max_capability: balanced, max_tool_scope: triage }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: filepath.Join(client, "sub"), ToolScope: "operator", Capability: "frontier"})
	if !v.ToolScopeRefused || v.ToolScope != "triage" {
		t.Errorf("operator was not clamped to triage inside a triage-capped tree: %+v", v)
	}
	if !v.CapabilityRefused || v.Capability != "balanced" {
		t.Errorf("frontier was not clamped to balanced inside a balanced-capped tree: %+v", v)
	}
	if v.Key != client {
		t.Errorf("matched ceiling %q, want the ancestor entry", v.Key)
	}
	// A sibling whose name shares the prefix is NOT inside it.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: client + "-old", ToolScope: "operator"}); v.ToolScopeRefused {
		t.Errorf("%s was treated as inside %s: %+v", client+"-old", client, v)
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
// the provider's own configured default, which on the desktop's Claude path is
// `opus[1m]` — chosen below where any ceiling can see it, whatever the matrix
// can read about the string. These pin the replacement tuple that closes it.

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

// THE CONTEXT-WINDOW SUFFIX. `opus[1m]` is `opus` with a 1M window rather than
// the standard 200K, and it is the desktop's shipped `claude.defaultModel` — so
// for as long as the named-model arm compared raw strings, the one Claude model
// a spawn reached by leaving `model` out entirely was the one no ceiling could
// judge. These pin that it is judged now, that judging it changes nothing about
// what the spawn actually carries, and that the normalizer eats the suffix and
// nothing else.

func TestTheCeilingJudgesAModelCarryingAWindowSuffix(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	// Exactly the assertions TestCheckSpawnJudgesTheModelAndNotOnlyTheLabel
	// makes for bare `opus`, repeated for the suffixed spelling. Same model,
	// same verdict, or the suffix is a way around the arm.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus[1m]"}); !v.CapabilityRefused {
		t.Errorf("`opus[1m]` with no effort was admitted under a frontier ceiling; bare `opus` is refused there, and the suffix asks for a bigger WINDOW, not a weaker model: %+v", v)
	}
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus[1m]", Effort: "high"}); v.CapabilityRefused {
		t.Errorf("`opus[1m]` at high effort was refused: naming the effort narrows the reading to deep_reviewer/frontier for the suffixed spelling exactly as it does for the bare one: %+v", v)
	}
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus[1m]", Effort: "max"}); !v.CapabilityRefused {
		t.Errorf("`opus[1m]` at max effort is frontier_max and was admitted under a frontier ceiling: %+v", v)
	}
	// And the `-1m` spelling of the same request, which the window contract
	// treats as the same marker.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus-1m"}); !v.CapabilityRefused {
		t.Errorf("the `-1m` spelling of the window request walked past the named-model arm: %+v", v)
	}
	// The suffix must not make an unknown model knowable, either.
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "some-unknown-model[1m]"}); v.CapabilityRefused {
		t.Errorf("a model this matrix never mentions was judged because it carried a suffix: %+v", v)
	}
}

// THE VERDICT IS IDENTICAL WITH AND WITHOUT THE SUFFIX, across the whole model
// vocabulary the shipped profiles use. Written as a parity sweep rather than a
// list of expected verdicts so it keeps holding when the profiles change: the
// claim is "the window suffix is not part of the model", and a parity check is
// that claim stated directly.
func TestTheWindowSuffixDoesNotChangeAnyVerdict(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	for _, model := range []string{"opus", "sonnet", "fable", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"} {
		for _, provider := range []string{"claude", "codex"} {
			for _, effort := range []string{"", "high", "max"} {
				bare := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: provider, Model: model, Effort: effort})
				for _, suffix := range []string{"[1m]", "-1m"} {
					got := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: provider, Model: model + suffix, Effort: effort})
					if got.CapabilityRefused != bare.CapabilityRefused || got.Capability != bare.Capability || got.Model != bare.Model {
						t.Errorf("%s %s%s (effort %q) is judged differently from %s %s: refused %v/%v, clamped to %q/%q, replaced with %q/%q",
							provider, model, suffix, effort, provider, model,
							got.CapabilityRefused, bare.CapabilityRefused, got.Capability, bare.Capability, got.Model, bare.Model)
					}
				}
			}
		}
	}
}

// NORMALIZED FOR THE COMPARISON, NOWHERE ELSE. Getting this wrong would be worse
// than the gap it closes: a ceiling that strips `[1m]` from what the provider is
// handed silently drops every dispatch from 1M to 200K, and nothing surfaces it
// until an agent runs out of room. The verdict is a CLAMP, so on the admitted
// path it names no model at all and the caller's string is what travels.
func TestAnAdmittedWindowSuffixIsNotRewritten(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: frontier_plus, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus[1m]", Effort: "high"})
	if v.Refused() {
		t.Fatalf("`opus[1m]` was refused under a frontier_plus ceiling: %+v", v)
	}
	if v.Model != "" {
		t.Errorf("the verdict named a model %q for a spawn it did not refuse — an admitted spawn keeps the string it sent, suffix and all, and anything written here would overwrite it: %+v", v.Model, v)
	}
}

// The model still names its own provider through the suffix. `opus[1m]` is a
// claude model for exactly the reason `opus` is, and answering it with a codex
// replacement because the active profile prefers codex would be a re-route
// nobody asked for.
func TestTheReplacementFollowsASuffixedModelsOwnProvider(t *testing.T) {
	m, err := Load("test.yaml", []byte("active_profile: mixed\nceilings:\n  default: { max_capability: balanced, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus", Model: "opus[1m]"})
	if v.Provider != "claude" {
		t.Errorf("a spawn naming `opus[1m]` and no provider was routed to %q: mixed's balanced is codex, but the model said claude: %+v", v.Provider, v)
	}
}

// BOTH SIDES OF THE COMPARISON ARE NORMALIZED, so one model needs one entry
// whichever spelling a routing.yaml uses. Without the profile half, the entry
// below is a model the matrix mentions and the arm cannot find, which is the
// original bug pointing the other way.
func TestAProfileEntryMayCarryTheSuffixToo(t *testing.T) {
	m, err := Load("test.yaml", []byte(
		"active_profile: anthropic_only\n"+
			"profiles:\n  anthropic_only:\n    frontier_max: { provider: claude, model: \"opus[1m]\", effort: max }\n"+
			"ceilings:\n  default: { max_capability: frontier, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus", Effort: "max"})
	if !v.CapabilityRefused {
		t.Errorf("`opus` at max effort was admitted under a frontier ceiling, but this matrix's frontier_max entry is `opus[1m]` — the same model, spelled with the window it wants: %+v", v)
	}
}

// THE ANSWER TO "what happens to the window request when the model is replaced".
// The substituted model runs the window ITS OWN entry implies, so a profile that
// spells the suffix keeps it and one that does not, does not. Pasting the
// refused model's `[1m]` onto the replacement would invent an id the matrix
// never names, and it would be nonsense on a provider with no such vocabulary.
func TestTheReplacementCarriesTheMatrixEntrysOwnWindow(t *testing.T) {
	m, err := Load("test.yaml", []byte(
		"active_profile: anthropic_only\n"+
			"profiles:\n  anthropic_only:\n    balanced: { provider: claude, model: \"sonnet[1m]\", effort: high }\n"+
			"ceilings:\n  default: { max_capability: balanced, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus", Provider: "claude", Model: "opus[1m]"})
	if !v.CapabilityRefused {
		t.Fatalf("frontier_plus was not refused under a balanced ceiling: %+v", v)
	}
	if v.Model != "sonnet[1m]" {
		t.Errorf("the replacement is %q, not the `sonnet[1m]` this matrix spells for balanced — a substituted model takes the window its own entry asks for: %+v", v.Model, v)
	}
	joined := strings.Join(v.Because, " | ")
	if strings.Contains(joined, "does not carry over") {
		t.Errorf("the verdict warned that a window request was dropped while replacing one 1M model with another: %v", v.Because)
	}
}

// And when the replacement does NOT carry a window, the caller is told. It is a
// second thing they asked for and did not get, on an axis escalationScrubbed
// cannot express: that list records `model` was taken, not that a 1M window went
// with it.
func TestADroppedWindowRequestIsSaidOutLoud(t *testing.T) {
	m, err := Load("test.yaml", []byte("active_profile: anthropic_only\nceilings:\n  default: { max_capability: balanced, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_plus", Provider: "claude", Model: "opus[1m]"})
	if !v.CapabilityRefused || v.Model == "" {
		t.Fatalf("expected a clamp with a replacement model: %+v", v)
	}
	if _, suffix := splitModelWindowSuffix(v.Model); suffix != "" {
		t.Fatalf("this case needs a replacement with no window suffix, and got %q", v.Model)
	}
	joined := strings.Join(v.Because, " | ")
	if !strings.Contains(joined, "[1m]") || !strings.Contains(joined, "does not carry over") {
		t.Errorf("the 1M request was dropped without a word: %v", v.Because)
	}
}

// A spawn that neither declares a capability nor names a model is the shape this
// whole gap was reached by: the provider fills `model` in from
// `claude.defaultModel` AFTER the gate, one layer below anything the ceiling can
// see. The clamp has to name the replacement, and this pins that the arm above
// it does not accidentally make an unnamed model judgeable.
func TestAnOmittedModelIsStillNotJudgedByTheNamedModelArm(t *testing.T) {
	m, err := Load("test.yaml", []byte("ceilings:\n  default: { max_capability: balanced, max_tool_scope: operator }\n"))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude"}); v.CapabilityRefused {
		t.Errorf("a spawn naming no model at all was refused by the named-model arm — there is no model to read, and the matrix makes no claim about one: %+v", v)
	}
}

// TestTheNamedModelArmSeesAModelThatOnlyEverAppearsAsAnAlternative is a
// correctness hole, not a nicety.
//
// The named-model arm exists because a caller that declares a modest capability
// while NAMING a strong model would otherwise walk around the ceiling entirely —
// the ceiling would govern a label. A model that appears only inside an
// `alternatives:` list is a model this matrix genuinely resolves: the router
// hands it out the moment the primary is unusable. So a scan that read primaries
// alone would find no reading for it, report `ok == false`, and admit it — the
// same hole, reopened one field lower down.
//
// MUTATION GUARD: delete the alternatives loop in capabilityOfModel and the
// first half of this test fails; delete it in providerOfModel and the second
// half does.
func TestTheNamedModelArmSeesAModelThatOnlyEverAppearsAsAnAlternative(t *testing.T) {
	m, err := Load("test.yaml", []byte(`
profiles:
  mixed:
    frontier_max:
      alternatives:
        - { provider: claude, model: opus-secret, effort: max }
ceilings:
  default: { max_capability: frontier, max_tool_scope: operator }
`))
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	// The precondition: the model exists ONLY as an alternative. If some other
	// entry ever names it, this test is measuring that entry instead.
	for pname, prof := range m.Profiles {
		for cname, a := range prof {
			if matchableModel(a.Model) == "opus-secret" {
				t.Fatalf("%s.%s names opus-secret as a PRIMARY — the precondition is gone", pname, cname)
			}
		}
	}

	rank, capName, ok := m.capabilityOfModel("claude", "opus-secret", "max")
	if !ok {
		t.Fatalf("the matrix has no reading at all for a model it will happily spawn — the named-model arm is blind to every alternative")
	}
	if capName != "frontier_max" || rank != m.RankOf("frontier_max") {
		t.Errorf("read as %q (rank %d), want frontier_max", capName, rank)
	}

	v := m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Provider: "claude", Model: "opus-secret", Effort: "max"})
	if !v.CapabilityRefused {
		t.Fatalf("a frontier_max model was admitted under a frontier ceiling because it only appears in an alternatives list: %+v", v)
	}

	// providerOfModel, the second scan: a spawn that names the model and NO
	// provider must still be clamped onto claude's own ladder rather than onto
	// whatever the active profile happens to prefer.
	if got := m.providerOfModel("opus-secret"); got != "claude" {
		t.Errorf("providerOfModel(opus-secret) = %q, want claude — a model the matrix names only as a fallover still plainly belongs to a provider", got)
	}
	v = m.CheckSpawn(SpawnRequest{CanonicalCwd: "/x", Capability: "frontier_max", Model: "opus-secret"})
	if !v.CapabilityRefused {
		t.Fatalf("the declared arm did not clamp: %+v", v)
	}
	if v.Provider != "claude" {
		t.Errorf("the clamp moved the spawn to %q — a model with one plain provider must not change harness on the way down: %+v", v.Provider, v)
	}
}
