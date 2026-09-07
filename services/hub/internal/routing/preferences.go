package routing

// Preferences are a sparse, typed overlay. The trusted document is only read;
// security metadata never crosses this API or comes from the managed layer.
import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"time"
)

type AlternativePreference struct {
	Provider  string `json:"provider"`
	Model     string `json:"model"`
	Effort    string `json:"effort,omitempty"`
	MinEffort string `json:"minEffort,omitempty"`
	Fresh     bool   `json:"fresh,omitempty"`
	Enabled   *bool  `json:"enabled,omitempty"`
}
type AssignmentPatch struct {
	Provider     *string                  `json:"provider,omitempty"`
	Model        *string                  `json:"model,omitempty"`
	Effort       *string                  `json:"effort,omitempty"`
	MinEffort    *string                  `json:"minEffort,omitempty"`
	Fresh        *bool                    `json:"fresh,omitempty"`
	Enabled      *bool                    `json:"enabled,omitempty"`
	Alternatives *[]AlternativePreference `json:"alternatives,omitempty"`
}
type ProviderPreference struct {
	Enabled *bool `json:"enabled,omitempty"`
}
type ModesPatch struct {
	Global    *string           `json:"global,omitempty"`
	Providers map[string]string `json:"providers,omitempty"`
}
type ModeShiftPatch struct {
	Roles                  map[string]string `json:"roles,omitempty"`
	EffortStep             *int              `json:"effortStep,omitempty"`
	EffortStepCapabilities *[]string         `json:"effortStepCapabilities,omitempty"`
}
type SpendDownPatch struct {
	TimeToResetMinutes        *float64 `json:"timeToResetMinutes,omitempty"`
	MinRemainingPct           *float64 `json:"minRemainingPct,omitempty"`
	MaxForecastPctOfRemaining *float64 `json:"maxForecastPctOfRemaining,omitempty"`
}
type HealthPatch struct {
	YellowAtUsedPct *float64 `json:"yellowAtUsedPct,omitempty"`
	RedAtUsedPct    *float64 `json:"redAtUsedPct,omitempty"`
}
type BootstrapPatch struct {
	MinElapsedPct     *float64 `json:"minElapsedPct,omitempty"`
	ExpectedOffsetPct *float64 `json:"expectedOffsetPct,omitempty"`
}
type SevenDayPatch struct {
	Curve             *string  `json:"curve,omitempty"`
	Timezone          *string  `json:"timezone,omitempty"`
	WeekendWeight     *float64 `json:"weekendWeight,omitempty"`
	Weekend           *string  `json:"weekend,omitempty"`
	WeekendReservePct *float64 `json:"weekendReservePct,omitempty"`
}
type PacingPatch struct {
	Enabled               *bool           `json:"enabled,omitempty"`
	ConserveAtRatio       *float64        `json:"conserveAtRatio,omitempty"`
	BlockSpendDownAtRatio *float64        `json:"blockSpendDownAtRatio,omitempty"`
	Bootstrap             *BootstrapPatch `json:"bootstrap,omitempty"`
	SevenDay              *SevenDayPatch  `json:"sevenDay,omitempty"`
}
type ThresholdsPatch struct {
	SpendDown *SpendDownPatch `json:"spendDown,omitempty"`
	Health    *HealthPatch    `json:"health,omitempty"`
	Pacing    *PacingPatch    `json:"pacing,omitempty"`
}
type PreferencesPatch struct {
	ActiveProfile   *string                               `json:"activeProfile,omitempty"`
	Roles           map[string]string                     `json:"roles,omitempty"`
	Profiles        map[string]map[string]AssignmentPatch `json:"profiles,omitempty"`
	Providers       map[string]ProviderPreference         `json:"providers,omitempty"`
	Modes           *ModesPatch                           `json:"modes,omitempty"`
	ModeShifts      map[string]ModeShiftPatch             `json:"modeShifts,omitempty"`
	Thresholds      *ThresholdsPatch                      `json:"thresholds,omitempty"`
	ForecastWeights map[string]float64                    `json:"forecastWeights,omitempty"`
}
type SafePolicy struct {
	ActiveProfile   string                        `json:"activeProfile"`
	Roles           map[string]string             `json:"roles"`
	Profiles        map[string]Profile            `json:"profiles"`
	Providers       map[string]ProviderPreference `json:"providers"`
	Modes           Modes                         `json:"modes"`
	ModeShifts      map[string]ModeShift          `json:"modeShifts"`
	Thresholds      Thresholds                    `json:"thresholds"`
	ForecastWeights map[string]float64            `json:"forecastWeights"`
}
type CatalogSnapshot struct {
	State      string         `json:"state"`
	ObservedAt int64          `json:"observedAt,omitempty"`
	Models     []CatalogModel `json:"models,omitempty"`
}

// SnapshotCatalog must only read a bounded cache, never launch a CLI or login.
type SnapshotCatalog interface {
	Snapshot() map[string]CatalogSnapshot
}
type PreferencesValidation struct {
	CatalogChecked bool     `json:"catalogChecked"`
	Valid          bool     `json:"valid"`
	CatalogPending bool     `json:"catalogPending"`
	Issues         []Issue  `json:"issues"`
	ChangedPaths   []string `json:"changedPaths"`
}
type PreferencesView struct {
	SchemaVersion int                        `json:"schemaVersion"`
	Revision      string                     `json:"revision"`
	Configurable  bool                       `json:"configurable"`
	Defaults      SafePolicy                 `json:"defaults"`
	Inherited     SafePolicy                 `json:"inherited"`
	Overrides     PreferencesPatch           `json:"overrides"`
	Effective     SafePolicy                 `json:"effective"`
	SourceByPath  map[string]string          `json:"sourceByPath"`
	ManagedFields []string                   `json:"managedFields"`
	Catalog       map[string]CatalogSnapshot `json:"catalog"`
	Validation    PreferencesValidation      `json:"validation"`
	Warning       string                     `json:"warning,omitempty"`
}
type PreferencesRequest struct {
	BaseRevision string           `json:"baseRevision"`
	Patch        PreferencesPatch `json:"patch"`
}
type PreferencesResult struct {
	Status     string                `json:"status"` // applied, valid, invalid, conflict, unavailable
	View       PreferencesView       `json:"view"`
	Validation PreferencesValidation `json:"validation"`
}
type preferencesFile struct {
	SchemaVersion int              `json:"schemaVersion"`
	Patch         PreferencesPatch `json:"patch"`
}

// DecodePreferences rejects unknown fields, null, duplicate keys, trailing data
// and excessive payloads. Case-sensitive field checking prevents encoding/json's
// otherwise case-insensitive names from becoming a second patch spelling.
func DecodePreferences(raw []byte, out any) error {
	if len(raw) == 0 {
		raw = []byte(`{}`)
	}
	if len(raw) > 256<<10 {
		return fmt.Errorf("routing preferences payload too large")
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	var walk func() error
	walk = func() error {
		t, e := d.Token()
		if e != nil {
			return e
		}
		if t == nil {
			return fmt.Errorf("null is not a preference; use reset")
		}
		if delim, ok := t.(json.Delim); ok {
			if delim == '{' {
				seen := map[string]bool{}
				for d.More() {
					k, e := d.Token()
					if e != nil {
						return e
					}
					key := k.(string)
					if seen[key] {
						return fmt.Errorf("duplicate field %s", key)
					}
					seen[key] = true
					if e = walk(); e != nil {
						return e
					}
				}
			} else if delim == '[' {
				for d.More() {
					if e := walk(); e != nil {
						return e
					}
				}
			} else {
				return fmt.Errorf("invalid JSON")
			}
			_, e = d.Token()
			return e
		}
		return nil
	}
	if err := walk(); err != nil {
		return err
	}
	if _, err := d.Token(); err != io.EOF {
		return fmt.Errorf("expected one JSON object")
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(out); err != nil {
		return err
	}
	var fields any
	if err := json.Unmarshal(raw, &fields); err != nil {
		return err
	}
	return exactFields(fields, reflect.TypeOf(out).Elem())
}
func exactFields(v any, t reflect.Type) error {
	for t.Kind() == reflect.Pointer {
		t = t.Elem()
	}
	switch t.Kind() {
	case reflect.Struct:
		m, ok := v.(map[string]any)
		if !ok {
			return fmt.Errorf("expected object")
		}
		for k, val := range m {
			found := false
			for i := 0; i < t.NumField(); i++ {
				f := t.Field(i)
				if strings.Split(f.Tag.Get("json"), ",")[0] == k {
					found = true
					if e := exactFields(val, f.Type); e != nil {
						return e
					}
					break
				}
			}
			if !found {
				return fmt.Errorf("unknown field %s", k)
			}
		}
	case reflect.Map:
		if m, ok := v.(map[string]any); ok {
			for _, val := range m {
				if e := exactFields(val, t.Elem()); e != nil {
					return e
				}
			}
		}
	case reflect.Slice:
		if a, ok := v.([]any); ok {
			for _, val := range a {
				if e := exactFields(val, t.Elem()); e != nil {
					return e
				}
			}
		}
	}
	return nil
}
func safePolicy(m *Matrix) SafePolicy {
	providers := map[string]ProviderPreference{}
	for p, v := range m.Providers {
		providers[p] = ProviderPreference{v.Enabled}
	}
	return SafePolicy{m.ActiveProfile, m.Roles, m.Profiles, providers, m.Modes, m.ModeShifts, m.Thresholds, m.ForecastWeights}
}
func jsonObject(v any) map[string]any {
	b, _ := json.Marshal(v)
	var m map[string]any
	_ = json.Unmarshal(b, &m)
	return m
}
func leafPaths(m map[string]any, prefix string, out map[string]any) {
	for k, v := range m {
		p := k
		if prefix != "" {
			p = prefix + "." + k
		}
		if obj, ok := v.(map[string]any); ok {
			leafPaths(obj, p, out)
		} else {
			out[p] = v
		}
	}
}
func changedPaths(a, b any) []string {
	x, y := map[string]any{}, map[string]any{}
	leafPaths(jsonObject(a), "", x)
	leafPaths(jsonObject(b), "", y)
	keys := map[string]bool{}
	for k := range x {
		keys[k] = true
	}
	for k := range y {
		keys[k] = true
	}
	out := []string{}
	for k := range keys {
		if !reflect.DeepEqual(x[k], y[k]) {
			out = append(out, k)
		}
	}
	sort.Strings(out)
	return out
}
func (s *Service) preferencesPath() string {
	if s.path == "" {
		return ""
	}
	return filepath.Join(filepath.Dir(s.path), "routing-preferences.json")
}
func (s *Service) readSourcesLocked() ([]byte, []byte, string, error) {
	if s.path == "" {
		return nil, nil, "", nil
	}
	host, e := os.ReadFile(s.path)
	if os.IsNotExist(e) {
		host = nil
		e = nil
	}
	if e != nil {
		return nil, nil, "", fmt.Errorf("host policy cannot be read")
	}
	pref, e := os.ReadFile(s.preferencesPath())
	if os.IsNotExist(e) {
		pref = nil
		e = nil
	}
	if e != nil {
		return nil, nil, "", fmt.Errorf("managed preferences cannot be read")
	}
	return host, pref, preferencesRevision(host, pref), nil
}
func preferencesRevision(host, pref []byte) string {
	h := sha256.New()
	h.Write([]byte("routing-preferences-v1\x00"))
	h.Write(host)
	h.Write([]byte{0})
	h.Write(pref)
	return fmt.Sprintf("%x", h.Sum(nil))
}

func composePreferences(host []byte, patch PreferencesPatch) (*Matrix, *Matrix, error) {
	base, err := Load("", host)
	if err != nil {
		return nil, nil, fmt.Errorf("host policy is invalid; last valid policy retained")
	}
	// Validate named keys before merging. No new vocabulary, ranks, or profiles.
	for p, rows := range patch.Profiles {
		if _, ok := base.Profiles[p]; !ok {
			return nil, nil, fmt.Errorf("unknown profile %q", p)
		}
		for c := range rows {
			if _, ok := base.Profiles[p][c]; !ok {
				return nil, nil, fmt.Errorf("unknown assignment %q", c)
			}
		}
	}
	for r, c := range patch.Roles {
		if _, ok := base.Roles[r]; !ok {
			return nil, nil, fmt.Errorf("unknown role %q", r)
		}
		if base.RankOf(c) < 0 {
			return nil, nil, fmt.Errorf("unknown capability %q", c)
		}
	}
	for p := range patch.Providers {
		if _, ok := base.Providers[p]; !ok {
			return nil, nil, fmt.Errorf("unknown provider %q", p)
		}
	}
	if patch.Modes != nil {
		for p := range patch.Modes.Providers {
			if _, ok := base.Providers[p]; !ok {
				return nil, nil, fmt.Errorf("unknown provider %q", p)
			}
		}
	}
	for mode, shift := range patch.ModeShifts {
		if _, ok := base.ModeShifts[mode]; !ok {
			return nil, nil, fmt.Errorf("unknown shift %q", mode)
		}
		for r, c := range shift.Roles {
			if _, ok := base.Roles[r]; !ok || base.RankOf(c) < 0 {
				return nil, nil, fmt.Errorf("unknown shift role or capability")
			}
		}
		if shift.EffortStepCapabilities != nil {
			for _, c := range *shift.EffortStepCapabilities {
				if base.RankOf(c) < 0 {
					return nil, nil, fmt.Errorf("unknown shift capability")
				}
			}
		}
	}
	for phase := range patch.ForecastWeights {
		if _, ok := base.ForecastWeights[phase]; !ok {
			return nil, nil, fmt.Errorf("unknown forecast phase %q", phase)
		}
	}
	merged := deepMerge(jsonObject(safePolicy(base)), jsonObject(patch))
	raw, _ := json.Marshal(merged)
	var p SafePolicy
	if err = json.Unmarshal(raw, &p); err != nil {
		return nil, nil, err
	}
	next := *base
	next.preferenceAuthority = base
	next.ActiveProfile = p.ActiveProfile
	next.Roles = p.Roles
	next.Profiles = p.Profiles
	next.Modes = p.Modes
	next.ModeShifts = p.ModeShifts
	next.Thresholds = p.Thresholds
	next.ForecastWeights = p.ForecastWeights
	next.Providers = map[string]Provider{}
	for k, v := range base.Providers {
		v.Enabled = p.Providers[k].Enabled
		next.Providers[k] = v
	}
	if err = preferencesSecurity(base, &next); err != nil {
		return nil, nil, err
	}
	// Host-only issues may predate this edit. Only newly introduced structural
	// issues refuse preferences; unrelated unavailable host entries remain editable.
	old := map[Issue]bool{}
	for _, i := range base.Issues {
		old[i] = true
	}
	next.Issues = validate(&next)
	for _, i := range next.Issues {
		if !old[i] {
			return nil, nil, fmt.Errorf("%s", i)
		}
	}
	return base, &next, nil
}
func preferencesSecurity(base, next *Matrix) error {
	if !reflect.DeepEqual(base.Thresholds, next.Thresholds) {
		h := next.Thresholds.Health
		sp := next.Thresholds.SpendDown
		if h.YellowAtUsedPct < 0 || h.YellowAtUsedPct >= h.RedAtUsedPct || h.RedAtUsedPct > 100 {
			return fmt.Errorf("health thresholds must satisfy 0 <= yellow < red <= 100")
		}
		if sp.TimeToResetMinutes < 0 || sp.TimeToResetMinutes > 10080 || sp.MinRemainingPct < 0 || sp.MinRemainingPct > 100 || sp.MaxForecastPctOfRemaining < 0 || sp.MaxForecastPctOfRemaining > 100 {
			return fmt.Errorf("spend-down thresholds require percentages 0..100 and reset minutes 0..10080")
		}
	}
	for mode, shift := range next.ModeShifts {
		if shift.EffortStep != base.ModeShifts[mode].EffortStep && (shift.EffortStep < -10 || shift.EffortStep > 10) {
			return fmt.Errorf("%s effort step must be between -10 and 10", mode)
		}
	}

	for profile, rows := range next.Profiles {
		for cap, a := range rows {
			inherited := base.Profiles[profile][cap]
			if reflect.DeepEqual(inherited, a) {
				continue
			}
			if inherited.Fresh && !a.Fresh {
				return fmt.Errorf("profiles.%s.%s: host freshness floor cannot be lowered", profile, cap)
			}
			for _, candidate := range withAlternatives(a) {
				for _, alt := range candidate.Alternatives {
					if len(alt.Alternatives) > 0 {
						return fmt.Errorf("nested alternatives are not supported")
					}
				}
				// Existing trusted tuples are grandfathered only within their original row.
				same := false
				floor := inherited
				for _, old := range withAlternatives(inherited) {
					if candidate.Provider == old.Provider && candidate.Model == old.Model && candidate.Effort == old.Effort {
						same = true
						floor = old
						if old.Fresh && !candidate.Fresh {
							return fmt.Errorf("profiles.%s.%s: host candidate freshness cannot be lowered", profile, cap)
						}
						break
					}
				}
				if !same {
					rank, known := preferenceAssignmentRank(base, candidate)
					if !known || rank > base.RankOf(cap) {
						return fmt.Errorf("profiles.%s.%s: model/effort needs host classification at this capability or below", profile, cap)
					}
				}
				if candidate.MinEffort != floor.MinEffort && floor.MinEffort != "" {
					oldFloor, oldOK := effortRung(floor.Provider, floor.MinEffort)
					newFloor, newOK := effortRung(candidate.Provider, candidate.MinEffort)
					if !oldOK || !newOK || newFloor < oldFloor {
						return fmt.Errorf("profiles.%s.%s: minimum effort cannot lower the host floor", profile, cap)
					}
				}
				if inherited.Fresh && !candidate.Fresh {
					return fmt.Errorf("profiles.%s.%s: alternatives must preserve host freshness", profile, cap)
				}
			}
		}
	}
	// Role remapping/profile switching must not erase a host freshness requirement.
	for role := range base.Roles {
		_, _, required := base.freshRequirement(role, "")
		_, _, fresh := next.freshRequirement(role, "")
		if required && !fresh {
			return fmt.Errorf("roles.%s: host freshness floor cannot be lowered", role)
		}
	}
	return nil
}
func parsePreferences(raw []byte) (PreferencesPatch, error) {
	if len(raw) == 0 {
		return PreferencesPatch{}, nil
	}
	var f preferencesFile
	if e := DecodePreferences(raw, &f); e != nil {
		return f.Patch, e
	}
	if f.SchemaVersion != 1 {
		return f.Patch, fmt.Errorf("unsupported routing preferences schema version")
	}
	return f.Patch, nil
}
func (s *Service) catalogSnapshotLocked() map[string]CatalogSnapshot {
	out := map[string]CatalogSnapshot{}
	if c, ok := s.cat.(SnapshotCatalog); ok {
		out = c.Snapshot()
	}
	for p := range knownProviders {
		if _, ok := out[p]; !ok {
			out[p] = CatalogSnapshot{State: "unknown"}
		}
	}
	return out
}
func (s *Service) preferencesViewLocked() PreferencesView {
	s.reloadIfChangedLocked()
	def, _ := Defaults()
	base := s.preferencesBase
	if base == nil {
		base = def
	}
	v := PreferencesView{SchemaVersion: 1, Revision: s.preferencesRevision, Configurable: s.path != "" && s.preferencesWarning == "", Defaults: safePolicy(def), Inherited: safePolicy(base), Effective: safePolicy(s.matrix), Overrides: s.preferencesPatch, SourceByPath: map[string]string{}, Catalog: s.catalogSnapshotLocked(), Warning: s.preferencesWarning, ManagedFields: []string{}}
	defaults, host, managed := map[string]any{}, map[string]any{}, map[string]any{}
	leafPaths(jsonObject(v.Defaults), "", defaults)
	leafPaths(jsonObject(v.Inherited), "", host)
	leafPaths(jsonObject(v.Overrides), "", managed)
	for k, val := range host {
		v.SourceByPath[k] = "shipped"
		if !reflect.DeepEqual(defaults[k], val) {
			v.SourceByPath[k] = "host"
		}
	}
	for k := range managed {
		v.SourceByPath[k] = "managed"
		v.ManagedFields = append(v.ManagedFields, k)
	}
	sort.Strings(v.ManagedFields)
	issues := []Issue{}
	for _, issue := range s.matrix.Issues {
		if strings.HasPrefix(issue.Where, "profiles.") || strings.HasPrefix(issue.Where, "roles.") || strings.HasPrefix(issue.Where, "thresholds.") || strings.HasPrefix(issue.Where, "modes.") || strings.HasPrefix(issue.Where, "mode_shifts.") || strings.HasPrefix(issue.Where, "forecast_weights.") || issue.Where == "active_profile" {
			issues = append(issues, issue)
		}
	}
	v.Validation = PreferencesValidation{CatalogChecked: s.matrix.CatalogChecked, Valid: s.preferencesWarning == "" && len(issues) == 0, Issues: issues, ChangedPaths: []string{}, CatalogPending: s.catalogPending}
	return v
}
func (s *Service) Preferences() PreferencesView {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.preferencesViewLocked()
}
func (s *Service) UpdatePreferences(req PreferencesRequest, action string) (PreferencesResult, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	v := s.preferencesViewLocked()
	result := PreferencesResult{View: v, Status: "unavailable", Validation: PreferencesValidation{Issues: []Issue{}, ChangedPaths: []string{}}}
	host, _, rev, err := s.readSourcesLocked()
	if err != nil {
		return result, err
	}
	if req.BaseRevision == "" || req.BaseRevision != rev {
		result.Status = "conflict"
		return result, nil
	}
	if !v.Configurable {
		return result, nil
	}
	patch := req.Patch
	if action != "reset" {
		b, _ := json.Marshal(deepMerge(jsonObject(s.preferencesPatch), jsonObject(req.Patch)))
		if err = DecodePreferences(b, &patch); err != nil {
			return result, err
		}
	}
	base, next, err := composePreferences(host, patch)
	result.Status = "invalid"
	if err != nil {
		result.Validation.Issues = append(result.Validation.Issues, Issue{Where: "preferences", Detail: err.Error()})
		return result, nil
	}
	result.Validation.ChangedPaths = changedPaths(safePolicy(s.matrix), safePolicy(next))
	if action != "reset" {
		result.Validation.Issues, result.Validation.CatalogPending = changedCatalogIssues(s.matrix, next, v.Catalog)
	}
	if len(result.Validation.Issues) > 0 || result.Validation.CatalogPending {
		return result, nil
	}
	result.Validation.Valid = true
	result.Validation.CatalogChecked = true
	if action == "validate" {
		result.Status = "valid"
		return result, nil
	}
	raw, _ := json.MarshalIndent(preferencesFile{1, patch}, "", "  ")
	raw = append(raw, '\n')
	tmp, err := createPreferencesTemp(filepath.Dir(s.preferencesPath()))
	if err != nil {
		return result, fmt.Errorf("cannot prepare routing preferences")
	}
	name := tmp.Name()
	defer os.Remove(name)
	if err = tmp.Chmod(0o600); err == nil {
		_, err = tmp.Write(raw)
	}
	if err == nil {
		err = tmp.Sync()
	}
	closeErr := tmp.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		return result, fmt.Errorf("cannot write routing preferences")
	}
	_, _, latest, err := s.readSourcesLocked()
	if err != nil {
		return result, err
	}
	if latest != rev {
		result.Status = "conflict"
		result.View = s.preferencesViewLocked()
		return result, nil
	}
	if err = os.Rename(name, s.preferencesPath()); err != nil {
		return result, fmt.Errorf("cannot install routing preferences")
	}
	if dir, e := os.Open(filepath.Dir(name)); e == nil {
		_ = dir.Sync()
		_ = dir.Close()
	}
	if len(host) > 0 {
		base.Source = s.path
		next.Source = s.path
	}
	s.preferencesBase = base
	s.preferencesPatch = patch
	s.installWithCachedCatalogLocked(next)
	s.preferencesRevision = preferencesRevision(host, raw)
	s.preferencesWarning = ""
	result.Status = "applied"
	result.View = s.preferencesViewLocked()
	return result, nil
}
func changedCatalogIssues(old, next *Matrix, cat map[string]CatalogSnapshot) ([]Issue, bool) {
	issues := []Issue{}
	pending := false
	for p, rows := range next.Profiles {
		for c, a := range rows {
			prior := old.Profiles[p][c]
			for _, candidate := range withAlternatives(a) {
				same := false
				for _, before := range withAlternatives(prior) {
					if candidate.Provider == before.Provider && candidate.Model == before.Model && candidate.Effort == before.Effort && candidate.MinEffort == before.MinEffort {
						same = true
						break
					}
				}
				if same {
					continue
				}
				where := "profiles." + p + "." + c
				snap := cat[candidate.Provider]
				if snap.State == "unknown" || snap.State == "" || snap.ObservedAt == 0 || time.Since(time.UnixMilli(snap.ObservedAt)) > 10*time.Minute {
					pending = true
					continue
				}
				found := false
				for _, model := range snap.Models {
					if model.ID != candidate.Model {
						continue
					}
					found = true
					for _, effort := range []string{candidate.Effort, candidate.MinEffort} {
						if effort == "" || len(model.EffortLevels) == 0 {
							continue
						}
						ok := false
						for _, level := range model.EffortLevels {
							if effort == level {
								ok = true
							}
						}
						if !ok {
							issues = append(issues, Issue{where, "effort is not supported by the cached model catalog"})
						}
					}
				}
				if !found {
					issues = append(issues, Issue{where, "model is unavailable in the cached provider catalog"})
				}
			}
		}
	}
	return issues, pending
}

type cachedPreferenceCatalog map[string]CatalogSnapshot

func (c cachedPreferenceCatalog) Models(p string) ([]CatalogModel, error) {
	v := c[p]
	if v.State != "available" || time.Since(time.UnixMilli(v.ObservedAt)) > 10*time.Minute {
		return nil, fmt.Errorf("catalog unknown")
	}
	return v.Models, nil
}

// A lower effort on an already classified model inherits the nearest trusted
// upper rung's rank. A new model still requires host classification; omitting
// effort takes the strongest reading, exactly as the canonical spawn gate does.
func preferenceAssignmentRank(base *Matrix, a Assignment) (int, bool) {
	if r, _, ok := base.capabilityOfModel(a.Provider, a.Model, a.Effort); ok {
		return r, true
	}
	rung, ok := effortRung(a.Provider, a.Effort)
	if !ok {
		return 0, false
	}
	upper, best, found := 1000, 0, false
	for _, rows := range base.Profiles {
		for cap, row := range rows {
			for _, old := range withAlternatives(row) {
				if a.Provider != old.Provider || matchableModel(a.Model) != matchableModel(old.Model) {
					continue
				}
				r, known := effortRung(old.Provider, old.Effort)
				if !known || r < rung {
					continue
				}
				rank := base.RankOf(cap)
				if rank < 0 {
					rank = UnrankedCapabilityStrength
				}
				if r < upper {
					upper, best, found = r, rank, true
				} else if r == upper && rank > best {
					best = rank
				}
			}
		}
	}
	return best, found
}
func (s *Service) installWithCachedCatalogLocked(m *Matrix) {
	s.installLocked(m)
	found := ValidateAgainstCatalog(m, cachedPreferenceCatalog(s.catalogSnapshotLocked()))
	if len(found) > 0 {
		next := *m
		next.Issues = append(append([]Issue(nil), m.Issues...), found...)
		s.matrix = &next
		s.catalogIssues = found
	}
}
