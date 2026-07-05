package plugin

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestSettingsValidation(t *testing.T) {
	base := func(s ...SettingDef) Manifest {
		return Manifest{ID: "x", APIVersion: APIVersion, Settings: s}
	}

	ok := base(
		SettingDef{Key: "vimMode", Label: "Vim mode", Type: SettingBoolean, Default: false},
		SettingDef{Key: "tabSize", Label: "Tab size", Type: SettingNumber, Default: 2},
		SettingDef{Key: "theme", Label: "Theme", Type: SettingSelect, Options: []string{"dark", "light"}, Default: "dark"},
	)
	if err := ok.Validate(); err != nil {
		t.Fatalf("valid settings rejected: %v", err)
	}

	cases := []struct {
		name string
		mf   Manifest
	}{
		{"empty key", base(SettingDef{Type: SettingBoolean})},
		{"unknown type", base(SettingDef{Key: "k", Type: "color"})},
		{"select without options", base(SettingDef{Key: "k", Type: SettingSelect})},
		{"duplicate key", base(
			SettingDef{Key: "k", Type: SettingBoolean},
			SettingDef{Key: "k", Type: SettingString},
		)},
	}
	for _, c := range cases {
		if err := c.mf.Validate(); err == nil {
			t.Errorf("%s: expected validation error", c.name)
		}
	}
}

func TestSettingsUnmarshal(t *testing.T) {
	var caps []SettingDef
	in := `[
	  {"key":"vimMode","label":"Vim mode","type":"boolean","default":false},
	  {"key":"tabSize","label":"Tab size","type":"number","default":2,"help":"Spaces per indent"},
	  {"key":"wrap","label":"Soft wrap","type":"select","options":["off","on"],"default":"off"}
	]`
	if err := json.Unmarshal([]byte(in), &caps); err != nil {
		t.Fatal(err)
	}
	if len(caps) != 3 || caps[0].Key != "vimMode" || caps[2].Options[1] != "on" {
		t.Fatalf("unexpected parse: %+v", caps)
	}
}

// settingsManifest is the shared fixture for the value-persistence tests: one of
// each setting type, with defaults on all but the string.
func settingsManifest(dir string) Manifest {
	return Manifest{
		ID: "acme.editor", APIVersion: APIVersion, Dir: dir,
		Settings: []SettingDef{
			{Key: "vimMode", Label: "Vim mode", Type: SettingBoolean, Default: false},
			{Key: "tabSize", Label: "Tab size", Type: SettingNumber, Default: float64(2)},
			{Key: "greeting", Label: "Greeting", Type: SettingString},
			{Key: "theme", Label: "Theme", Type: SettingSelect, Options: []string{"dark", "light"}, Default: "dark"},
		},
	}
}

// GetSettings on a plugin with nothing persisted returns exactly the manifest
// defaults — and only for settings that declare one (the string setting has no
// default, so it's absent rather than nil).
func TestGetSettingsDefaultsMerge(t *testing.T) {
	m := loadedManager(t, newFakeRegistrar(), settingsManifest(t.TempDir()))

	got, err := m.GetSettings("acme.editor")
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]any{"vimMode": false, "tabSize": float64(2), "theme": "dark"}
	if len(got) != len(want) {
		t.Fatalf("defaults = %+v, want %+v", got, want)
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("default %q = %v (%T), want %v (%T)", k, got[k], got[k], v, v)
		}
	}
	if _, present := got["greeting"]; present {
		t.Errorf("greeting has no default, should be absent, got %v", got["greeting"])
	}
}

// SetSettings validates, persists, and returns the merged view; the overlay
// survives a fresh manager reading the same directory (round-trip on disk).
func TestSetSettingsPersistsAndMerges(t *testing.T) {
	dir := t.TempDir()
	m := loadedManager(t, newFakeRegistrar(), settingsManifest(dir))

	merged, err := m.SetSettings("acme.editor", map[string]any{
		"vimMode": true,
		"tabSize": float64(4),
		"theme":   "light",
	})
	if err != nil {
		t.Fatal(err)
	}
	if merged["vimMode"] != true || merged["tabSize"] != float64(4) || merged["theme"] != "light" {
		t.Fatalf("merged after set = %+v", merged)
	}

	// The on-disk overlay stores only the changed values, not the full schema.
	raw, err := os.ReadFile(filepath.Join(dir, settingsValuesFile))
	if err != nil {
		t.Fatalf("overlay not written: %v", err)
	}
	var overlay map[string]any
	if err := json.Unmarshal(raw, &overlay); err != nil {
		t.Fatal(err)
	}
	if _, hasGreeting := overlay["greeting"]; hasGreeting {
		t.Errorf("unset key should not be persisted: %+v", overlay)
	}

	// A brand-new manager over the same dir reads the persisted values back.
	m2 := loadedManager(t, newFakeRegistrar(), settingsManifest(dir))
	got, err := m2.GetSettings("acme.editor")
	if err != nil {
		t.Fatal(err)
	}
	if got["vimMode"] != true || got["tabSize"] != float64(4) || got["theme"] != "light" {
		t.Fatalf("reloaded settings = %+v", got)
	}
}

// A nil value deletes a key, reverting it to the manifest default.
func TestSetSettingsNilRevertsToDefault(t *testing.T) {
	dir := t.TempDir()
	m := loadedManager(t, newFakeRegistrar(), settingsManifest(dir))

	if _, err := m.SetSettings("acme.editor", map[string]any{"theme": "light"}); err != nil {
		t.Fatal(err)
	}
	got, err := m.SetSettings("acme.editor", map[string]any{"theme": nil})
	if err != nil {
		t.Fatal(err)
	}
	if got["theme"] != "dark" {
		t.Fatalf("theme after revert = %v, want default \"dark\"", got["theme"])
	}
}

// Validation rejects wrong types, non-member select values, and unknown keys —
// and a rejected write persists nothing.
func TestSetSettingsValidationRejects(t *testing.T) {
	cases := []struct {
		name    string
		partial map[string]any
	}{
		{"wrong type boolean", map[string]any{"vimMode": "yes"}},
		{"wrong type number", map[string]any{"tabSize": "four"}},
		{"select not a member", map[string]any{"theme": "solarized"}},
		{"select wrong type", map[string]any{"theme": 3}},
		{"unknown key", map[string]any{"nope": true}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			dir := t.TempDir()
			m := loadedManager(t, newFakeRegistrar(), settingsManifest(dir))
			if _, err := m.SetSettings("acme.editor", c.partial); err == nil {
				t.Fatalf("%s: expected rejection", c.name)
			}
			// Nothing should have been written on a rejected all-or-nothing write.
			if _, statErr := os.Stat(filepath.Join(dir, settingsValuesFile)); statErr == nil {
				t.Errorf("%s: overlay file written despite validation failure", c.name)
			}
		})
	}
}

// A rejected write leaves a previously-persisted overlay untouched (atomicity:
// the valid key in the same partial must not sneak through).
func TestSetSettingsRejectionIsAtomic(t *testing.T) {
	dir := t.TempDir()
	m := loadedManager(t, newFakeRegistrar(), settingsManifest(dir))
	if _, err := m.SetSettings("acme.editor", map[string]any{"tabSize": float64(8)}); err != nil {
		t.Fatal(err)
	}
	// vimMode is valid but theme is not: the whole write must fail and tabSize stays 8.
	if _, err := m.SetSettings("acme.editor", map[string]any{"vimMode": true, "theme": "bad"}); err == nil {
		t.Fatal("expected rejection for the invalid theme value")
	}
	got, _ := m.GetSettings("acme.editor")
	if got["tabSize"] != float64(8) {
		t.Errorf("tabSize = %v, want unchanged 8", got["tabSize"])
	}
	if got["vimMode"] != false {
		t.Errorf("vimMode = %v, want unchanged default false (partial must not have applied)", got["vimMode"])
	}
}

// SetSettings emits plugin.settings.changed carrying the plugin id and full
// merged values.
func TestSetSettingsEmitsChangeEvent(t *testing.T) {
	dir := t.TempDir()
	cap := newCapture()
	m := NewManager(cap, newFakeRegistrar())
	m.mu.Lock()
	m.plugins["acme.editor"] = &loaded{manifest: settingsManifest(dir)}
	m.mu.Unlock()

	if _, err := m.SetSettings("acme.editor", map[string]any{"vimMode": true}); err != nil {
		t.Fatal(err)
	}
	ev := cap.waitFor(t, "plugin.settings.changed")

	var payload struct {
		ID     string         `json:"id"`
		Values map[string]any `json:"values"`
	}
	if err := json.Unmarshal(ev.Data, &payload); err != nil {
		t.Fatal(err)
	}
	if payload.ID != "acme.editor" {
		t.Errorf("event id = %q, want acme.editor", payload.ID)
	}
	if payload.Values["vimMode"] != true {
		t.Errorf("event values = %+v, want vimMode:true", payload.Values)
	}
}

// GetSettings / SetSettings on an unloaded plugin error rather than touching the
// filesystem, and a loaded plugin with no directory can't persist.
func TestSettingsUnloadedAndNoDir(t *testing.T) {
	m := NewManager(newCapture(), newFakeRegistrar())
	if _, err := m.GetSettings("ghost"); err == nil {
		t.Error("GetSettings on unloaded plugin should error")
	}
	if _, err := m.SetSettings("ghost", map[string]any{"x": 1}); err == nil {
		t.Error("SetSettings on unloaded plugin should error")
	}

	m.mu.Lock()
	m.plugins["nodir"] = &loaded{manifest: Manifest{ID: "nodir", APIVersion: APIVersion,
		Settings: []SettingDef{{Key: "x", Type: SettingNumber}}}}
	m.mu.Unlock()
	if _, err := m.SetSettings("nodir", map[string]any{"x": float64(1)}); err == nil {
		t.Error("SetSettings on a plugin with no dir should error")
	}
}
