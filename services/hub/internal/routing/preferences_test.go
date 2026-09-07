package routing

import (
	"encoding/json"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"
)

type preferenceCatalog map[string]CatalogSnapshot

func (c preferenceCatalog) Models(p string) ([]CatalogModel, error) {
	panic("preferences must not probe a provider")
}
func (c preferenceCatalog) Snapshot() map[string]CatalogSnapshot { return c }
func preferenceFixture(t *testing.T) (*Service, string, []byte) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "routing.yaml")
	host := []byte("# preserve these comments exactly\nactive_profile: codex_only\nfuture_unknown: {kept: true}\nceilings:\n  default: {max_capability: balanced, max_tool_scope: view}\n")
	if e := os.WriteFile(path, host, 0600); e != nil {
		t.Fatal(e)
	}
	cat := preferenceCatalog{"codex": {State: "available", ObservedAt: time.Now().UnixMilli(), Models: []CatalogModel{{ID: "gpt-5.6-terra", EffortLevels: []string{"high"}}, {ID: "gpt-5.6-luna"}}}}
	return New(path, cat), path, host
}
func prefsRequest(t *testing.T, s *Service, patch string) PreferencesRequest {
	t.Helper()
	var p PreferencesPatch
	if e := DecodePreferences([]byte(patch), &p); e != nil {
		t.Fatal(e)
	}
	return PreferencesRequest{BaseRevision: s.Preferences().Revision, Patch: p}
}
func TestPreferencesApplyPersistResetAndConsume(t *testing.T) {
	s, path, host := preferenceFixture(t)
	before := s.Matrix()
	req := prefsRequest(t, s, `{"roles":{"scout":"cheap"},"thresholds":{"health":{"yellowAtUsedPct":65}}}`)
	validated, e := s.UpdatePreferences(req, "validate")
	if e != nil || validated.Status != "valid" {
		t.Fatalf("validate %+v %v", validated, e)
	}
	if s.Matrix() != before {
		t.Fatal("validation installed candidate")
	}
	if _, e = os.Stat(s.preferencesPath()); !os.IsNotExist(e) {
		t.Fatal("validation wrote sidecar")
	}
	got, e := s.UpdatePreferences(req, "save")
	if e != nil || got.Status != "applied" {
		t.Fatalf("save %+v %v", got, e)
	}
	d := Select(s.Matrix(), limits.Snapshot{}, nil, nil, time.Now(), Request{Role: "scout"})
	if d.Model != "gpt-5.6-luna" || d.Capability != "cheap" {
		t.Fatalf("consumer ignored save: %+v", d)
	}
	if s.Matrix().Thresholds.Health.YellowAtUsedPct != 65 || s.Matrix().Thresholds.Health.RedAtUsedPct != before.Thresholds.Health.RedAtUsedPct {
		t.Fatal("sparse threshold not preserved")
	}
	restart := New(path, nil)
	if !reflect.DeepEqual(safePolicy(s.Matrix()), safePolicy(restart.Matrix())) {
		t.Fatal("restart did not restore managed policy")
	}
	stat, e := os.Stat(s.preferencesPath())
	if e != nil || (runtime.GOOS != "windows" && stat.Mode().Perm() != 0600) {
		t.Fatalf("sidecar not private: %v %v", stat, e)
	}
	v := s.Preferences()
	if v.SourceByPath["roles.scout"] != "managed" || v.SourceByPath["activeProfile"] != "host" || v.Defaults.ActiveProfile != "mixed" {
		t.Fatalf("sources %+v", v.SourceByPath)
	}
	reset, e := s.UpdatePreferences(PreferencesRequest{BaseRevision: v.Revision}, "reset")
	if e != nil || reset.Status != "applied" || reset.View.Effective.ActiveProfile != "codex_only" || len(reset.View.ManagedFields) != 0 {
		t.Fatalf("reset %+v %v", reset, e)
	}
	raw, _ := os.ReadFile(path)
	if string(raw) != string(host) {
		t.Fatal("trusted YAML changed")
	}
}
func TestPreferencesConflictAndInvalidSources(t *testing.T) {
	s, path, _ := preferenceFixture(t)
	req := prefsRequest(t, s, `{"roles":{"scout":"cheap"}}`)
	var wg sync.WaitGroup
	statuses := make(chan string, 2)
	for i := 0; i < 2; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			r, e := s.UpdatePreferences(req, "save")
			if e != nil {
				t.Error(e)
			}
			statuses <- r.Status
		}()
	}
	wg.Wait()
	close(statuses)
	counts := map[string]int{}
	for v := range statuses {
		counts[v]++
	}
	if counts["applied"] != 1 || counts["conflict"] != 1 {
		t.Fatalf("CAS %v", counts)
	}
	req = prefsRequest(t, s, `{"roles":{"fixer":"cheap"}}`)
	disk, _ := os.ReadFile(s.preferencesPath())
	if e := os.WriteFile(path, []byte("active_profile: mixed\n"), 0600); e != nil {
		t.Fatal(e)
	}
	r, e := s.UpdatePreferences(req, "save")
	if e != nil || r.Status != "conflict" {
		t.Fatalf("host CAS %s %v", r.Status, e)
	}
	after, _ := os.ReadFile(s.preferencesPath())
	if string(disk) != string(after) {
		t.Fatal("conflict wrote disk")
	}
	live := s.Matrix()
	_ = os.WriteFile(path, []byte("profiles: [broken"), 0600)
	if s.ReloadIfChanged() || s.Matrix() != live || s.Preferences().Warning == "" {
		t.Fatal("invalid source displaced live policy or not reported")
	}
	_ = os.WriteFile(path, []byte("active_profile: mixed\n"), 0600)
	s.ReloadIfChanged()
	_ = os.WriteFile(s.preferencesPath(), []byte(`{"schemaVersion":2,"patch":{}}`), 0600)
	live = s.Matrix()
	if s.ReloadIfChanged() || s.Matrix() != live || s.Preferences().Configurable {
		t.Fatal("newer sidecar reinterpreted")
	}
}
func TestPreferencesRejectsSecurityBypassAndUnknownFields(t *testing.T) {
	for _, raw := range []string{`{"ceilings":{}}`, `{"capabilities":[]}`, `{"capabilityRanks":{}}`, `{"path":"/tmp/other"}`, `{"rawYAML":"x"}`, `{"activeProfile":null}`, `{"activeProfile":"mixed","activeProfile":"codex_only"}`, `{"ActiveProfile":"mixed"}`, `{"profiles":{"mixed":{"cheap":{"alternatives":[{"provider":"codex","model":"x","alternatives":[]}]}}}}`} {
		var p PreferencesPatch
		if e := DecodePreferences([]byte(raw), &p); e == nil {
			t.Errorf("accepted forbidden input %s", raw)
		}
	}
	s, _, _ := preferenceFixture(t)
	for _, patch := range []string{
		`{"profiles":{"mixed":{"cheap":{"provider":"claude","model":"fable","effort":""}}}}`,
		`{"profiles":{"codex_only":{"reviewer":{"fresh":false}}}}`,
		`{"roles":{"reviewer":"cheap"},"modeShifts":{"spend_down":{"roles":{"reviewer":"cheap"}}}}`,
		`{"profiles":{"new_profile":{}}}`, `{"roles":{"scout":"invented"}}`,
		`{"modes":{"global":"typo"}}`, `{"modes":{"providers":{"codex":"CONSERVE"}}}`,
		`{"forecastWeights":{"implementation":-1}}`, `{"forecastWeights":{"review":1000001}}`,
		`{"thresholds":{"health":{"yellowAtUsedPct":99,"redAtUsedPct":20}}}`,
	} {
		before := s.Matrix()
		r, e := s.UpdatePreferences(prefsRequest(t, s, patch), "save")
		if e == nil && r.Status != "invalid" {
			t.Errorf("accepted %s: %+v", patch, r)
		}
		if s.Matrix() != before {
			t.Fatal("invalid changed live")
		}
	}
	v := s.Preferences()
	raw, _ := json.Marshal(v)
	for _, word := range []string{"ceilings", "capabilityRanks", "maxToolScope", "routing.yaml", s.Path()} {
		if strings.Contains(string(raw), word) {
			t.Errorf("projection leaks %q", word)
		}
	}
}
func TestPreferencesUnknownCatalogOnlyBlocksChangedAssignments(t *testing.T) {
	s, _, _ := preferenceFixture(t)
	s.cat = preferenceCatalog{}
	req := prefsRequest(t, s, `{"profiles":{"codex_only":{"balanced":{"provider":"codex","model":"gpt-5.6-luna","effort":""}}}}`)
	r, e := s.UpdatePreferences(req, "save")
	if e != nil || r.Status != "invalid" || !r.Validation.CatalogPending {
		t.Fatalf("unknown must be pending: %+v %v", r, e)
	}
	s.cat = preferenceCatalog{"codex": {State: "unavailable", ObservedAt: time.Now().UnixMilli()}}
	r, e = s.UpdatePreferences(req, "save")
	if e != nil || r.Status != "invalid" || r.Validation.CatalogPending || len(r.Validation.Issues) == 0 {
		t.Fatalf("unavailable %+v %v", r, e)
	}
	r, e = s.UpdatePreferences(prefsRequest(t, s, `{"roles":{"scout":"cheap"}}`), "save")
	if e != nil || r.Status != "applied" {
		t.Fatalf("unrelated edit blocked %+v %v", r, e)
	}
	r, e = s.UpdatePreferences(PreferencesRequest{BaseRevision: r.View.Revision}, "reset")
	if e != nil || r.Status != "applied" {
		t.Fatalf("reset blocked %+v %v", r, e)
	}
}
func TestPreferencesRetainsHostModelClassificationAndCeilings(t *testing.T) {
	s, _, _ := preferenceFixture(t)
	before := s.Matrix().CheckSpawn(SpawnRequest{CanonicalCwd: t.TempDir(), Provider: "codex", Model: "gpt-5.6-sol", Effort: "xhigh", Capability: "cheap", ToolScope: "operator"})
	r, e := s.UpdatePreferences(prefsRequest(t, s, `{"profiles":{"codex_only":{"frontier_plus":{"model":"gpt-5.6-terra","effort":"high"},"frontier_max":{"model":"gpt-5.6-terra","effort":"high"}}}}`), "save")
	if e != nil || r.Status != "applied" {
		t.Fatalf("lowering rows %+v %v", r, e)
	}
	after := s.Matrix().CheckSpawn(SpawnRequest{CanonicalCwd: t.TempDir(), Provider: "codex", Model: "gpt-5.6-sol", Effort: "xhigh", Capability: "cheap", ToolScope: "operator"})
	if before.CapabilityRefused != after.CapabilityRefused || !after.ToolScopeRefused {
		t.Fatalf("ceiling weakened: before %+v after %+v", before, after)
	}
	if rank, _, ok := s.Matrix().capabilityOfModel("codex", "gpt-5.6-sol", "xhigh"); !ok || rank != 5 {
		t.Fatalf("lost host classification %d %v", rank, ok)
	}
}

// Shared Go/TS fixture pins the projection without copying machine policy.
// Regenerate with UPDATE_ROUTING_FIXTURE=1 go test ./internal/routing -run TestPreferencesWireFixture.
func TestPreferencesWireFixture(t *testing.T) {
	s := New("", nil)
	v := s.Preferences()
	v.Configurable = true
	v.Revision = "fixture-revision"
	for provider := range v.Catalog {
		seen := map[string]bool{}
		models := []CatalogModel{}
		for _, profile := range sortedKeys(s.matrix.Profiles) {
			for _, cap := range sortedKeys(s.matrix.Profiles[profile]) {
				for _, a := range withAlternatives(s.matrix.Profiles[profile][cap]) {
					if a.Provider == provider && !seen[a.Model] {
						seen[a.Model] = true
						ladder, _ := EffortLadder(provider)
						models = append(models, CatalogModel{ID: a.Model, EffortLevels: ladder})
					}
				}
			}
		}
		state := "unknown"
		if len(models) > 0 {
			state = "available"
		}
		v.Catalog[provider] = CatalogSnapshot{State: state, Models: models}
	}
	raw, _ := json.MarshalIndent(v, "", "  ")
	raw = append(raw, '\n')
	path := filepath.Join("testdata", "preferences-view.json")
	if os.Getenv("UPDATE_ROUTING_FIXTURE") == "1" {
		if err := os.WriteFile(path, raw, 0600); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil || string(raw) != string(want) {
		t.Fatal("routing wire fixture drift: regenerate with UPDATE_ROUTING_FIXTURE=1", err)
	}
}

func TestPreferencesLowerEffortUsesTrustedUpperRung(t *testing.T) {
	s, _, _ := preferenceFixture(t)
	s.cat = preferenceCatalog{"codex": {State: "available", ObservedAt: time.Now().UnixMilli(), Models: []CatalogModel{{ID: "gpt-5.6-sol", EffortLevels: []string{"medium", "high", "xhigh"}}}}}
	r, e := s.UpdatePreferences(prefsRequest(t, s, `{"profiles":{"codex_only":{"frontier":{"effort":"medium"}}}}`), "save")
	if e != nil || r.Status != "applied" {
		t.Fatalf("lower effort refused %s %v %v", r.Status, r.Validation.Issues, e)
	}
	if s.Matrix().Profiles["codex_only"]["frontier"].Effort != "medium" {
		t.Fatal("effort did not reach runtime")
	}
	r, e = s.UpdatePreferences(prefsRequest(t, s, `{"profiles":{"codex_only":{"cheap":{"model":"gpt-5.6-sol","effort":"medium"}}}}`), "save")
	if e != nil || r.Status != "invalid" {
		t.Fatalf("lower effort laundered model strength %s %v", r.Status, e)
	}
}

func TestPreferencesAfterHostFileDeletedBeforeRestart(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing.yaml")
	_ = New(path, nil)
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	s := New(path, nil)
	if !s.Preferences().Configurable {
		t.Fatal("deliberately absent host file disables inherited preferences after restart")
	}
	r, e := s.UpdatePreferences(prefsRequest(t, s, `{"roles":{"scout":"cheap"}}`), "save")
	if e != nil || r.Status != "applied" {
		t.Fatalf("save without host file %s %v", r.Status, e)
	}
	r, e = s.UpdatePreferences(PreferencesRequest{BaseRevision: r.View.Revision}, "reset")
	if e != nil || r.Status != "applied" || r.View.Effective.Roles["scout"] != r.View.Defaults.Roles["scout"] {
		t.Fatalf("reset did not reveal shipped baseline %s %v", r.Status, e)
	}
	if _, e := os.Stat(path); !os.IsNotExist(e) {
		t.Fatal("preferences recreated deleted host YAML")
	}
}

func TestPreferencesAdvancedFieldsReachSelection(t *testing.T) {
	t.Run("mode shifts effort and forecast", func(t *testing.T) {
		s, _, _ := preferenceFixture(t)
		r, e := s.UpdatePreferences(prefsRequest(t, s, `{"modes":{"global":"conserve"},"modeShifts":{"conserve":{"roles":{"scout":"balanced"},"effortStep":-1,"effortStepCapabilities":["balanced"]}},"forecastWeights":{"implementation":7}}`), "save")
		if e != nil || r.Status != "applied" {
			t.Fatalf("save %s %v %v", r.Status, r.Validation.Issues, e)
		}
		d := Select(s.Matrix(), limits.Snapshot{}, nil, nil, policyNow, Request{Role: "scout", ExpectedWork: []limits.Work{{Phase: "implementation", Count: 2}}})
		if d.Mode != ModeConserve || d.Capability != "balanced" || d.Effort != "medium" || d.Demand.Units != 14 {
			t.Fatalf("advanced controls did not reach selection %+v", d)
		}
	})
	t.Run("health thresholds", func(t *testing.T) {
		s, _, _ := preferenceFixture(t)
		snap := snapshotOf(t, "codex", "", map[string]winSpec{"five_hour": {used: 80, resets: 3 * time.Hour}, "seven_day": {used: 10, resets: 24 * time.Hour}})
		before := Select(s.Matrix(), snap, nil, nil, policyNow, Request{Role: "scout"})
		r, e := s.UpdatePreferences(prefsRequest(t, s, `{"thresholds":{"health":{"yellowAtUsedPct":60,"redAtUsedPct":75}}}`), "save")
		if e != nil || r.Status != "applied" {
			t.Fatal(r.Status, e)
		}
		after := Select(s.Matrix(), snap, nil, nil, policyNow, Request{Role: "scout"})
		if before.Capacity.Health == after.Capacity.Health || after.Capacity.Health != limits.HealthRed || after.Mode != ModeConserve {
			t.Fatalf("health threshold not consumed: before %s after %+v", before.Capacity.Health, after)
		}
	})
	t.Run("pace threshold", func(t *testing.T) {
		s, _, _ := preferenceFixture(t)
		snap := snapshotOf(t, "codex", "", map[string]winSpec{"five_hour": {used: 40, resets: 200 * time.Minute, minutes: 300}, "seven_day": {used: 10, resets: 24 * time.Hour}})
		before := Select(s.Matrix(), snap, nil, nil, policyNow, Request{Role: "scout"})
		r, e := s.UpdatePreferences(prefsRequest(t, s, `{"thresholds":{"pacing":{"conserveAtRatio":1.1,"blockSpendDownAtRatio":1.05}}}`), "save")
		if e != nil || r.Status != "applied" {
			t.Fatal(r.Status, e)
		}
		after := Select(s.Matrix(), snap, nil, nil, policyNow, Request{Role: "scout"})
		if before.Mode != ModeNormal || after.Mode != ModeConserve {
			t.Fatalf("pace threshold not consumed: %s -> %s", before.Mode, after.Mode)
		}
	})
	t.Run("provider disabled", func(t *testing.T) {
		s, _, _ := preferenceFixture(t)
		r, e := s.UpdatePreferences(prefsRequest(t, s, `{"providers":{"codex":{"enabled":false}}}`), "save")
		if e != nil || r.Status != "applied" {
			t.Fatal(r.Status, e)
		}
		if d := Select(s.Matrix(), limits.Snapshot{}, nil, nil, policyNow, Request{Role: "scout"}); d.Eligible {
			t.Fatalf("disabled provider ignored %+v", d)
		}
	})
	t.Run("assignment and ordered alternative", func(t *testing.T) {
		s, _, _ := preferenceFixture(t)
		s.cat = preferenceCatalog{"claude": {State: "available", ObservedAt: time.Now().UnixMilli(), Models: []CatalogModel{{ID: "sonnet", EffortLevels: []string{"high"}}}}}
		r, e := s.UpdatePreferences(prefsRequest(t, s, `{"profiles":{"codex_only":{"balanced":{"enabled":false,"alternatives":[{"provider":"claude","model":"sonnet","effort":"high"}]}}}}`), "save")
		if e != nil || r.Status != "applied" {
			t.Fatalf("save %s %v %v", r.Status, r.Validation.Issues, e)
		}
		d := Select(s.Matrix(), limits.Snapshot{}, nil, nil, policyNow, Request{Role: "scout"})
		if d.Provider != "claude" || d.Model != "sonnet" || d.Effort != "high" || d.FellOverFrom == nil {
			t.Fatalf("alternative not consumed %+v", d)
		}
	})
}

func TestPreferencesSourcesTrackExplicitEqualHostFields(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing.yaml")
	host := []byte("roles: {scout: balanced}\nmode_shifts:\n  conserve: {scout: cheap, effort_step: -1}\nthresholds:\n  pacing:\n    bootstrap: {min_elapsed_pct: 5}\n")
	if e := os.WriteFile(path, host, 0600); e != nil {
		t.Fatal(e)
	}
	v := New(path, nil).Preferences()
	for _, path := range []string{"roles.scout", "modeShifts.conserve.roles.scout", "modeShifts.conserve.effortStep", "thresholds.pacing.bootstrap.minElapsedPct"} {
		if v.SourceByPath[path] != "host" {
			t.Errorf("equal host value lost provenance at %s: %s", path, v.SourceByPath[path])
		}
	}
	if v.SourceByPath["roles.mechanical"] != "shipped" {
		t.Fatal("omitted field falsely attributed to host")
	}
}
