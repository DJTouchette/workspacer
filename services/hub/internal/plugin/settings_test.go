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

// secretManifest: one secret PAT-style setting beside a plain string.
func secretManifest(dir string) Manifest {
	return Manifest{
		ID: "acme.ci", APIVersion: APIVersion, Dir: dir,
		Settings: []SettingDef{
			{Key: "token", Label: "API token", Type: SettingString, Secret: true},
			{Key: "repo", Label: "Repository", Type: SettingString},
		},
	}
}

// Secret settings must be strings and must not ship a manifest default (the
// unguarded /plugins listing would leak it).
func TestSecretValidation(t *testing.T) {
	base := func(s ...SettingDef) Manifest {
		return Manifest{ID: "x", APIVersion: APIVersion, Settings: s}
	}
	valid := base(SettingDef{Key: "token", Type: SettingString, Secret: true})
	if err := valid.Validate(); err != nil {
		t.Fatalf("valid secret setting rejected: %v", err)
	}
	secretBool := base(SettingDef{Key: "token", Type: SettingBoolean, Secret: true})
	if err := secretBool.Validate(); err == nil {
		t.Error("secret boolean should be rejected")
	}
	secretDefault := base(SettingDef{Key: "token", Type: SettingString, Secret: true, Default: "hunter2"})
	if err := secretDefault.Validate(); err == nil {
		t.Error("secret with a default should be rejected")
	}
}

// The core secret contract: plaintext at rest and in the sidecar env, the
// placeholder on every read (Get, Set echo — the same map feeds the
// plugin.settings.changed broadcast).
func TestSecretRedactedOnReadsPlaintextInEnv(t *testing.T) {
	dir := t.TempDir()
	mf := secretManifest(dir)
	m := loadedManager(t, newFakeRegistrar(), mf)

	echoed, err := m.SetSettings("acme.ci", map[string]any{"token": "ghp_abc123", "repo": "o/r"})
	if err != nil {
		t.Fatal(err)
	}
	if echoed["token"] != SecretPlaceholder {
		t.Errorf("Set echo leaks the secret: %v", echoed["token"])
	}
	if echoed["repo"] != "o/r" {
		t.Errorf("non-secret redacted: %v", echoed["repo"])
	}

	got, err := m.GetSettings("acme.ci")
	if err != nil {
		t.Fatal(err)
	}
	if got["token"] != SecretPlaceholder {
		t.Errorf("GetSettings leaks the secret: %v", got["token"])
	}

	// At rest: plaintext (0600 overlay is the storage of record).
	raw, err := os.ReadFile(filepath.Join(dir, settingsValuesFile))
	if err != nil {
		t.Fatal(err)
	}
	var overlay map[string]any
	if err := json.Unmarshal(raw, &overlay); err != nil {
		t.Fatal(err)
	}
	if overlay["token"] != "ghp_abc123" {
		t.Errorf("overlay should store plaintext, got %v", overlay["token"])
	}

	// Sidecar env: plaintext — that delivery is the point of the secret.
	var env map[string]any
	if err := json.Unmarshal([]byte(m.settingsEnvJSON(mf)), &env); err != nil {
		t.Fatal(err)
	}
	if env["token"] != "ghp_abc123" {
		t.Errorf("WKS_SETTINGS env should carry plaintext, got %v", env["token"])
	}
}

// A client that reads settings and saves them all back must not clobber the
// stored secret with the placeholder; an empty string still clears it.
func TestSecretSentinelEchoIgnored(t *testing.T) {
	dir := t.TempDir()
	m := loadedManager(t, newFakeRegistrar(), secretManifest(dir))

	if _, err := m.SetSettings("acme.ci", map[string]any{"token": "ghp_abc123"}); err != nil {
		t.Fatal(err)
	}
	// Echo the redacted read back, alongside a real change.
	if _, err := m.SetSettings("acme.ci", map[string]any{"token": SecretPlaceholder, "repo": "o/r"}); err != nil {
		t.Fatal(err)
	}
	overlay := readSettingsOverlay(dir)
	if overlay["token"] != "ghp_abc123" {
		t.Errorf("sentinel echo overwrote the secret: %v", overlay["token"])
	}
	if overlay["repo"] != "o/r" {
		t.Errorf("real change alongside sentinel was dropped: %v", overlay["repo"])
	}

	// Explicit empty string clears; reads then report it unmasked-empty.
	if _, err := m.SetSettings("acme.ci", map[string]any{"token": ""}); err != nil {
		t.Fatal(err)
	}
	got, err := m.GetSettings("acme.ci")
	if err != nil {
		t.Fatal(err)
	}
	if got["token"] != "" {
		t.Errorf("cleared secret should read as empty, got %v", got["token"])
	}
}
