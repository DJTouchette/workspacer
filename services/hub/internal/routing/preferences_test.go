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
