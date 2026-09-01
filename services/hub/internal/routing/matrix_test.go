package routing

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

// --- the shipped default -----------------------------------------------------

// TestShippedDefaultLoadsClean is the guard on the one file everything else
// falls back to. A typo in routing.default.yaml would otherwise surface as a
// role that quietly resolves to nothing on a user's machine.
func TestShippedDefaultLoadsClean(t *testing.T) {
	m, err := Load("", nil)
	if err != nil {
		t.Fatalf("the shipped default does not load: %v", err)
	}
	if len(m.Issues) != 0 {
		for _, iss := range m.Issues {
			t.Errorf("shipped default has a validation issue: %s", iss)
		}
	}
	if m.Version != 1 {
		t.Errorf("version = %d, want 1", m.Version)
	}
	if m.ActiveProfile != "mixed" {
		t.Errorf("active_profile = %q, want mixed (the recommended default)", m.ActiveProfile)
	}
	if m.Source != "" {
		t.Errorf("Source = %q with no user file — it must name a FILE that was merged, or nothing", m.Source)
	}
}

// TestEveryRoleResolvesUnderEveryProfile is the completeness rule the whole
// design rests on: workflow logic names a capability, so a profile that cannot
// answer one leaves a role with nowhere to go.
func TestEveryRoleResolvesUnderEveryProfile(t *testing.T) {
	m, err := Load("", nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(m.Roles) == 0 || len(m.Profiles) == 0 {
		t.Fatalf("empty matrix: %d roles, %d profiles", len(m.Roles), len(m.Profiles))
	}
	for pname := range m.Profiles {
		for role, capability := range m.Roles {
			a, err := m.ResolveCapability(pname, capability)
			if err != nil {
				t.Errorf("profile %s, role %s: %v", pname, role, err)
				continue
			}
			if a.Provider == "" || a.Model == "" {
				t.Errorf("profile %s, role %s resolves to an incomplete assignment %+v", pname, role, a)
			}
		}
	}
}

// TestSameFamilyReviewersAreFresh pins section 23: a reviewer that inherits the
// implementer's conversation is not a reviewer. Where the profile keeps review
// inside the implementer's own provider, `fresh` is the only thing carrying
// independence.
func TestSameFamilyReviewersAreFresh(t *testing.T) {
	m, err := Load("", nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, pname := range []string{"codex_only", "anthropic_only"} {
		prof := m.Profiles[pname]
		for _, capability := range []string{"reviewer", "deep_reviewer"} {
			if !prof[capability].Fresh {
				t.Errorf("%s.%s must be fresh: it reviews work its own provider produced", pname, capability)
			}
		}
	}
}

// --- the merge ---------------------------------------------------------------

func TestUserFileMergesOverDefaultsPerKey(t *testing.T) {
	// One key changed inside one assignment. Everything around it must survive:
	// the provider and model of the same entry, the other entries of the same
	// profile, and every other profile.
	m, err := Load("routing.yaml", []byte(`
profiles:
  mixed:
    frontier:
      effort: xhigh
`))
	if err != nil {
		t.Fatal(err)
	}
	got, err := m.ResolveCapability("mixed", "frontier")
	if err != nil {
		t.Fatal(err)
	}
	if got.Effort != "xhigh" {
		t.Errorf("effort = %q, want the user's xhigh", got.Effort)
	}
	if got.Provider != "codex" || got.Model != "gpt-5.6-sol" {
		t.Errorf("the sibling keys of an edited leaf were lost: %+v — a WHOLESALE replace looks exactly like this", got)
	}
	if a, err := m.ResolveCapability("mixed", "frontier_plus"); err != nil || a.Model == "" {
		t.Errorf("an untouched capability stopped resolving: %+v %v", a, err)
	}
	if a, err := m.ResolveCapability("anthropic_only", "frontier"); err != nil || a.Provider != "claude" {
		t.Errorf("an untouched profile was disturbed: %+v %v", a, err)
	}
	if m.Source != "routing.yaml" {
		t.Errorf("Source = %q, want the file that was merged", m.Source)
	}
}

// TestAMergeCannotDelete is the reason this file is deep-merged rather than
// replaced. Handing over one role must not take the other ten away, and the same
// goes for a profile, a provider and the default ceiling.
func TestAMergeCannotDelete(t *testing.T) {
	base, err := Load("", nil)
	if err != nil {
		t.Fatal(err)
	}
	m, err := Load("routing.yaml", []byte(`
roles:
  scout: cheap
profiles:
  mixed:
    balanced: { model: gpt-5.6-luna }
providers:
  codex: { metered: false }
ceilings:
  /home/u/client: { max_capability: balanced, max_tool_scope: triage }
`))
	if err != nil {
		t.Fatal(err)
	}
	if len(m.Roles) != len(base.Roles) {
		t.Errorf("%d roles after naming one, want all %d — a wholesale replace deletes the rest", len(m.Roles), len(base.Roles))
	}
	if m.Roles["scout"] != "cheap" {
		t.Errorf("the one role the user did name did not apply: %q", m.Roles["scout"])
	}
	if len(m.Profiles["mixed"]) != len(base.Profiles["mixed"]) {
		t.Errorf("%d capabilities in mixed, want all %d", len(m.Profiles["mixed"]), len(base.Profiles["mixed"]))
	}
	if len(m.Providers) != len(base.Providers) {
		t.Errorf("%d providers, want all %d", len(m.Providers), len(base.Providers))
	}
	if _, ok := m.Ceilings[CeilingDefaultKey]; !ok {
		t.Error("adding a per-project ceiling deleted the default one — the one entry every unlisted directory depends on")
	}
	// The blocks the user never mentioned at all are untouched.
	if m.Thresholds.SpendDown.TimeToResetMinutes != base.Thresholds.SpendDown.TimeToResetMinutes {
		t.Error("an unmentioned threshold block changed")
	}
}

// TestDisablingIsExplicit — since a merge cannot delete, `enabled: false` is the
// only spelling for "take this out of service", and absent must not mean it.
func TestDisablingIsExplicit(t *testing.T) {
	m, err := Load("routing.yaml", []byte(`
profiles:
  mixed:
    frontier_plus: { enabled: false }
providers:
  codex: { enabled: false }
`))
	if err != nil {
		t.Fatal(err)
	}
	a, err := m.ResolveCapability("mixed", "frontier_plus")
	if err != nil {
		t.Fatal(err)
	}
	if a.IsEnabled() {
		t.Error("an explicit enabled:false did not disable the entry")
	}
	if a.Model == "" {
		t.Error("disabling an entry erased it — it must stay resolvable so the reason it is unavailable can be explained")
	}
	if p, _ := m.ProviderPolicy("codex"); p.IsEnabled() {
		t.Error("an explicit enabled:false did not disable the provider")
	}
	if p, _ := m.ProviderPolicy("claude"); !p.IsEnabled() {
		t.Error("an entry with no `enabled` key must be enabled — absence is not disablement")
	}
}

// TestUnresolvableRoleFallsBackToTheShippedDefault — never to nothing.
func TestUnresolvableRoleFallsBackToTheShippedDefault(t *testing.T) {
	m, err := Load("routing.yaml", []byte(`
active_profile: no_such_profile
roles:
  implementer: no_such_capability
`))
	if err != nil {
		t.Fatal(err)
	}
	r, err := m.ResolveRole("implementer")
	if err != nil {
		t.Fatalf("a role pointed at a capability nothing resolves must fall back, not fail: %v", err)
	}
	if !r.FellBack {
		t.Error("it fell back without saying so — a silent fallback is how a half-applied matrix hides")
	}
	if r.Profile != "mixed" || r.Capability != "frontier" || r.Assignment.Model == "" {
		t.Errorf("fallback resolved to %+v, want the shipped mixed/frontier", r)
	}
	// And it is reported, not just survived.
	if !hasIssueAt(m.Issues, "active_profile") || !hasIssueAt(m.Issues, "roles.implementer") {
		t.Errorf("neither arm was reported at load: %v", m.Issues)
	}
}

// TestSpecProviderNamesNormalize — the design spec writes vendor names; the
// spawn wire takes workspacer provider ids. A matrix pasted out of the spec has
// to work.
func TestSpecProviderNamesNormalize(t *testing.T) {
	m, err := Load("routing.yaml", []byte(`
profiles:
  mixed:
    frontier: { provider: openai, model: gpt-5.6-sol }
    reviewer: { provider: anthropic, model: sonnet }
providers:
  anthropic: { metered: true, when_unknown: yellow }
modes:
  providers: { openai: conserve }
`))
	if err != nil {
		t.Fatal(err)
	}
	if a, _ := m.ResolveCapability("mixed", "frontier"); a.Provider != "codex" {
		t.Errorf("openai normalized to %q, want codex", a.Provider)
	}
	if a, _ := m.ResolveCapability("mixed", "reviewer"); a.Provider != "claude" {
		t.Errorf("anthropic normalized to %q, want claude", a.Provider)
	}
	if _, ok := m.Providers["anthropic"]; ok {
		t.Error("an `anthropic` key survived in providers — it would then be a second, unreachable entry beside claude")
	}
	if got := m.ModeFor("codex"); got != "conserve" {
		t.Errorf("ModeFor(codex) = %q, want the conserve the user wrote against `openai`", got)
	}
	if len(m.Issues) != 0 {
		t.Errorf("normalized names must not be reported as unknown providers: %v", m.Issues)
	}
}

// TestUnknownProviderIsReportedNotRefused.
func TestUnknownProviderIsReportedNotRefused(t *testing.T) {
	m, err := Load("routing.yaml", []byte(`
profiles:
  mixed:
    cheap: { provider: gemini, model: whatever }
`))
	if err != nil {
		t.Fatalf("an unknown provider must not fail the load: %v", err)
	}
	if !hasIssueAt(m.Issues, "profiles.mixed.cheap") {
		t.Errorf("an unknown provider id went unreported: %v", m.Issues)
	}
}

// --- key reporting -----------------------------------------------------------

func TestAppliedAndUnrecognizedKeysAreNamed(t *testing.T) {
	m, err := Load("routing.yaml", []byte(`
active_profile: codex_only
thresholds:
  spend_down:
    time_to_reset_minutes: 45
    typo_key: 3
`))
	if err != nil {
		t.Fatal(err)
	}
	wantApplied := []string{
		"active_profile",
		"thresholds.spend_down.time_to_reset_minutes",
		"thresholds.spend_down.typo_key",
	}
	for _, w := range wantApplied {
		if !contains(m.Applied, w) {
			t.Errorf("key %q was taken from the file but not reported (Applied = %v)", w, m.Applied)
		}
	}
	if contains(m.Applied, "roles.scout") {
		t.Errorf("Applied names a key the user did not write: %v", m.Applied)
	}
	if !contains(m.Unrecognized, "thresholds.spend_down.typo_key") {
		t.Errorf("a key matching nothing in the defaults went unreported: %v", m.Unrecognized)
	}
	if contains(m.Unrecognized, "active_profile") {
		t.Errorf("a real key was reported as unrecognized: %v", m.Unrecognized)
	}
}

// --- ceilings ----------------------------------------------------------------

func TestCeilingForNearestAncestorThenDefault(t *testing.T) {
	work := filepath.Join(t.TempDir(), "Work")
	client := filepath.Join(work, "client")
	m, err := Load("routing.yaml", []byte(`
ceilings:
  `+strconv.Quote(work)+`: { max_capability: frontier, max_tool_scope: operator }
  `+strconv.Quote(client)+`: { max_capability: balanced, max_tool_scope: triage }
`))
	if err != nil {
		t.Fatal(err)
	}
	if c, key := m.CeilingFor(client); key != client || c.MaxToolScope != "triage" {
		t.Errorf("exact match lost: key=%q %+v", key, c)
	}
	if c, key := m.CeilingFor(filepath.Join(client, "deep", "nested")); key != client || c.MaxCapability != "balanced" {
		t.Errorf("nearest ancestor lost to a shallower one: key=%q %+v", key, c)
	}
	if _, key := m.CeilingFor(filepath.Join(work, "other")); key != work {
		t.Errorf("a sibling project matched %q, want the %q entry", key, work)
	}
	// The lookup is separator-terminated, not a string prefix: "<work>-old" is
	// not inside "<work>".
	if _, key := m.CeilingFor(work + "-old"); key != CeilingDefaultKey {
		t.Errorf("a directory whose NAME merely starts with a ceiling key matched %q — that is a prefix test, not containment", key)
	}
	if c, key := m.CeilingFor(filepath.Join(t.TempDir(), "scratch")); key != CeilingDefaultKey || c.MaxCapability == "" {
		t.Errorf("an unlisted directory got %q %+v, want the default entry", key, c)
	}
}

// --- catalog validation ------------------------------------------------------

type fakeCatalog struct {
	models map[string][]CatalogModel
	errs   map[string]error
	asked  []string
}

func (f *fakeCatalog) Models(provider string) ([]CatalogModel, error) {
	f.asked = append(f.asked, provider)
	if err, ok := f.errs[provider]; ok {
		return nil, err
	}
	return f.models[provider], nil
}

func TestCatalogValidationNamesAnUnknownModelAtLoad(t *testing.T) {
	m, err := Load("routing.yaml", []byte(`
active_profile: codex_only
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-retired, effort: high }
`))
	if err != nil {
		t.Fatal(err)
	}
	cat := &fakeCatalog{models: map[string][]CatalogModel{
		"codex": {
			{ID: "gpt-5.6-sol", EffortLevels: []string{"low", "medium", "high", "xhigh"}},
			{ID: "gpt-5.6-terra", EffortLevels: []string{"low", "medium", "high", "xhigh"}},
			{ID: "gpt-5.6-luna"},
		},
	}}
	issues := ValidateAgainstCatalog(m, cat)
	if !hasIssueAt(issues, "profiles.codex_only.frontier") {
		t.Fatalf("a model the installed CLI does not serve was not reported at load: %v", issues)
	}
	found := ""
	for _, iss := range issues {
		if iss.Where == "profiles.codex_only.frontier" {
			found = iss.Detail
		}
	}
	if !strings.Contains(found, "gpt-5.6-retired") || !strings.Contains(found, "gpt-5.6-sol") {
		t.Errorf("the report must name both the bad id and what IS available; got %q", found)
	}
}

func TestCatalogValidationChecksEffortWhenTheProviderReportsALadder(t *testing.T) {
	m, err := Load("routing.yaml", []byte(`
profiles:
  mixed:
    frontier: { provider: codex, model: gpt-5.6-sol, effort: max }
`))
	if err != nil {
		t.Fatal(err)
	}
	cat := &fakeCatalog{models: map[string][]CatalogModel{
		// codex's ladder has no "max" — that is claude's word. This is exactly
		// the cross-provider mix-up the per-provider ladders exist to catch.
		"codex":  {{ID: "gpt-5.6-sol", EffortLevels: []string{"low", "medium", "high", "xhigh"}}},
		"claude": {{ID: "sonnet"}, {ID: "opus"}, {ID: "fable"}},
	}}
	issues := ValidateAgainstCatalog(m, cat)
	if !hasIssueAt(issues, "profiles.mixed.frontier") {
		t.Errorf("an effort level the provider does not take was not reported: %v", issues)
	}
	// claude reports no ladder here, so its efforts must go UNVALIDATED rather
	// than be refused for not being in an empty list.
	for _, iss := range issues {
		if strings.HasPrefix(iss.Where, "profiles.mixed.reviewer") {
			t.Errorf("effort was validated against a provider that reported no ladder: %s", iss)
		}
	}
}

func TestAnUnreachableProviderDoesNotCondemnTheMatrix(t *testing.T) {
	m, err := Load("", nil)
	if err != nil {
		t.Fatal(err)
	}
	cat := &fakeCatalog{errs: map[string]error{
		"codex":  os.ErrNotExist,
		"claude": os.ErrNotExist,
	}}
	if issues := ValidateAgainstCatalog(m, cat); len(issues) != 0 {
		t.Errorf("a CLI that is not installed on this machine says nothing about whether the matrix is right; got %v", issues)
	}
	if ValidateAgainstCatalog(m, nil) != nil {
		t.Error("no catalog at all must mean no catalog findings")
	}
}

func TestTheShippedDefaultMatchesTheLiveCatalogShape(t *testing.T) {
	// The ids the shipped default names, against the catalogs as they are
	// observed today. This is what turns "we wrote plausible model ids" into a
	// claim something checks.
	m, err := Load("", nil)
	if err != nil {
		t.Fatal(err)
	}
	codexEfforts := []string{"minimal", "low", "medium", "high", "xhigh"}
	cat := &fakeCatalog{models: map[string][]CatalogModel{
		"codex": {
			{ID: "gpt-5.6-sol", EffortLevels: codexEfforts},
			{ID: "gpt-5.6-terra", EffortLevels: codexEfforts},
			{ID: "gpt-5.6-luna", EffortLevels: codexEfforts},
		},
		"claude": {
			{ID: "fable", EffortLevels: []string{"low", "medium", "high", "xhigh", "max"}},
			{ID: "opus", EffortLevels: []string{"low", "medium", "high", "xhigh", "max"}},
			{ID: "sonnet", EffortLevels: []string{"low", "medium", "high", "xhigh", "max"}},
			{ID: "haiku", EffortLevels: []string{"low", "medium", "high", "xhigh", "max"}},
		},
	}}
	if issues := ValidateAgainstCatalog(m, cat); len(issues) != 0 {
		for _, iss := range issues {
			t.Errorf("shipped default names something the catalogs do not serve: %s", iss)
		}
	}
}

// --- helpers -----------------------------------------------------------------

func hasIssueAt(issues []Issue, where string) bool {
	for _, iss := range issues {
		if iss.Where == where {
			return true
		}
	}
	return false
}

func contains(hay []string, needle string) bool {
	for _, h := range hay {
		if h == needle {
			return true
		}
	}
	return false
}

// TestChangedExcludesKeysThatRestateTheDefault. The seeded file starts out
// byte-identical to the shipped default, so after one hand edit the file still
// "carries" a hundred keys. If the load log named all of them the one that moved
// would be buried, which defeats the point of logging them at all.
func TestChangedExcludesKeysThatRestateTheDefault(t *testing.T) {
	// The whole shipped file, re-submitted as if the user had saved it untouched.
	m, err := Load("routing.yaml", DefaultBytes())
	if err != nil {
		t.Fatal(err)
	}
	if len(m.Applied) == 0 {
		t.Fatal("the shipped document carries no keys at all — the walk is not walking")
	}
	if len(m.Changed) != 0 {
		t.Errorf("re-saving the shipped default changed %d key(s): %v", len(m.Changed), m.Changed)
	}

	// Now one real edit on top of it.
	edited := strings.Replace(string(DefaultBytes()), "active_profile: mixed", "active_profile: codex_only", 1)
	if edited == string(DefaultBytes()) {
		t.Fatal("the edit did not apply — this test would then assert nothing")
	}
	m2, err := Load("routing.yaml", []byte(edited))
	if err != nil {
		t.Fatal(err)
	}
	if len(m2.Changed) != 1 || m2.Changed[0] != "active_profile" {
		t.Errorf("Changed = %v, want exactly [active_profile]", m2.Changed)
	}
	if len(m2.Applied) != len(m.Applied) {
		t.Errorf("Applied = %d keys, want the same %d the file carries either way", len(m2.Applied), len(m.Applied))
	}
}

// TestVendorAliasKeysFoldBeforeTheMerge pins the ordering the alias fold has to
// happen in, and it loops because the bug it guards is a Go map-order coin
// flip: one run proves nothing.
//
// The shipped defaults always carry `codex:` under both `providers:` and
// `modes.providers:`. A user file that spells it the spec's way (`openai:`)
// therefore merges into a document holding BOTH keys, and folding the alias
// AFTER the decode collapsed two entries onto one in randomized map order — so
// `modes.providers.openai: conserve` was honoured or silently thrown away
// depending on the run. A setting read and then discarded by a coin flip is
// worse than one never read, because half the evidence says it works.
func TestVendorAliasKeysFoldBeforeTheMerge(t *testing.T) {
	for i := 0; i < 50; i++ {
		m, err := Load("test.yaml", []byte(
			"modes:\n  providers:\n    openai: conserve\nproviders:\n  anthropic:\n    enabled: false\n"))
		if err != nil {
			t.Fatalf("load: %v", err)
		}
		if got := m.ModeFor("codex"); got != "conserve" {
			t.Fatalf("run %d: modes.providers.openai: conserve resolved to %q for codex — the alias lost the collision with the shipped `codex: auto`", i, got)
		}
		if pol, ok := m.ProviderPolicy("claude"); !ok || pol.IsEnabled() {
			t.Fatalf("run %d: providers.anthropic.enabled: false did not take claude out of service (%+v)", i, pol)
		}
		// The shipped values the alias entry did NOT mention must survive: the
		// fold renames a key, it does not replace the block.
		if pol, _ := m.ProviderPolicy("claude"); !pol.Metered || pol.WhenUnknown != "yellow" {
			t.Fatalf("run %d: the alias entry replaced claude's whole provider block instead of merging into it: %+v", i, pol)
		}
		// And the load log names the key that actually applied.
		if !contains(m.Changed, "modes.providers.codex") {
			t.Fatalf("run %d: Changed says %v — an operator reading the log must see the key that took effect", i, m.Changed)
		}
	}
}
