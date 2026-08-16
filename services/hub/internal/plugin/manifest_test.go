package plugin

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writePlugin(t *testing.T, root, name, json string) {
	t.Helper()
	dir := filepath.Join(root, name)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "plugin.json"), []byte(json), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestValidate(t *testing.T) {
	good := Manifest{ID: "x", APIVersion: "1"}
	if err := good.Validate(); err != nil {
		t.Fatalf("expected valid: %v", err)
	}
	cases := []Manifest{
		{APIVersion: "1"},                                                               // no id
		{ID: "x", APIVersion: "0"},                                                      // bad version
		{ID: "x", APIVersion: "1", Server: &ServerSpec{}},                               // server no command
		{ID: "x", APIVersion: "1", Panes: []PaneContribution{{Type: ""}}},               // empty pane type
		{ID: "x", APIVersion: "1", Panes: []PaneContribution{{Type: "a"}, {Type: "a"}}}, // dup
	}
	for i, c := range cases {
		if err := c.Validate(); err == nil {
			t.Errorf("case %d: expected error", i)
		}
	}
}

// Widgets are a separate contribution kind from panes, with the same
// serve-from-somewhere requirement and a closed size vocabulary.
func TestValidateWidgets(t *testing.T) {
	ok := Manifest{ID: "x", APIVersion: "1", UI: "ui",
		Widgets: []WidgetContribution{{ID: "lamp", Title: "Lamp", Sizes: []WidgetSize{WidgetSmall, WidgetLarge}}}}
	if err := ok.Validate(); err != nil {
		t.Fatalf("expected valid: %v", err)
	}

	cases := map[string]Manifest{
		"empty widget id": {ID: "x", APIVersion: "1", UI: "ui",
			Widgets: []WidgetContribution{{ID: ""}}},
		"duplicate widget id": {ID: "x", APIVersion: "1", UI: "ui",
			Widgets: []WidgetContribution{{ID: "a"}, {ID: "a"}}},
		"unknown size": {ID: "x", APIVersion: "1", UI: "ui",
			Widgets: []WidgetContribution{{ID: "a", Sizes: []WidgetSize{"huge"}}}},
		// Same rule as panes: a widget is a webview and needs an origin to load from.
		"widgets with nothing to serve them": {ID: "x", APIVersion: "1",
			Widgets: []WidgetContribution{{ID: "a"}}},
	}
	for name, mf := range cases {
		if err := mf.Validate(); err == nil {
			t.Errorf("%s: expected an error", name)
		}
	}
}

// An omitted "sizes" defaults to small — the least presumptuous footprint.
func TestWidgetSupportedSizes(t *testing.T) {
	bare := WidgetContribution{ID: "a"}
	if got := bare.SupportedSizes(); len(got) != 1 || got[0] != WidgetSmall {
		t.Errorf("default sizes = %v, want [small]", got)
	}
	if !bare.Supports(WidgetSmall) {
		t.Error("a bare widget must support small")
	}
	if bare.Supports(WidgetLarge) {
		t.Error("a bare widget must not claim large")
	}
	declared := WidgetContribution{ID: "b", Sizes: []WidgetSize{WidgetMedium, WidgetLarge}}
	if declared.Supports(WidgetSmall) {
		t.Error("small was not declared")
	}
	if !declared.Supports(WidgetLarge) {
		t.Error("large was declared")
	}
}

func TestLoadDir(t *testing.T) {
	root := t.TempDir()
	writePlugin(t, root, "tracker", `{
		"id":"acme.tracker","name":"Tracker","apiVersion":"1",
		"server":{"command":"tracker-bin","port":9100,"health":"/healthz"},
		"panes":[{"type":"acme.tracker","title":"Issues","icon":"📋","path":"/ui"}],
		"widgets":[{"id":"count","title":"Open issues","icon":"📋","path":"/widget/count","sizes":["small","medium"]}],
		"hotkeys":[{"id":"open","default":"ctrl+shift+i","command":"open-pane:acme.tracker"}]
	}`)
	writePlugin(t, root, "broken", `{ not json `)
	writePlugin(t, root, "wrongver", `{"id":"x","apiVersion":"99"}`)
	// A non-plugin dir (no plugin.json) should be ignored.
	if err := os.MkdirAll(filepath.Join(root, "notaplugin"), 0o755); err != nil {
		t.Fatal(err)
	}

	manifests, errs := LoadDir(root)
	if len(manifests) != 1 {
		t.Fatalf("got %d manifests, want 1", len(manifests))
	}
	if len(errs) != 2 {
		t.Fatalf("got %d errors, want 2 (broken json + wrong version)", len(errs))
	}
	m := manifests[0]
	if m.ID != "acme.tracker" || len(m.Panes) != 1 || m.Panes[0].Type != "acme.tracker" {
		t.Fatalf("unexpected manifest: %+v", m)
	}
	if len(m.Hotkeys) != 1 || m.Hotkeys[0].Command != "open-pane:acme.tracker" {
		t.Fatalf("hotkey not parsed: %+v", m.Hotkeys)
	}
	if len(m.Widgets) != 1 || m.Widgets[0].ID != "count" || m.Widgets[0].Path != "/widget/count" {
		t.Fatalf("widget not parsed: %+v", m.Widgets)
	}
	if got := m.Widgets[0].SupportedSizes(); len(got) != 2 || got[0] != WidgetSmall || got[1] != WidgetMedium {
		t.Fatalf("widget sizes not parsed: %v", got)
	}
	if m.Dir == "" {
		t.Error("Dir should be set by the loader")
	}
}

func TestLoadDirMissing(t *testing.T) {
	manifests, errs := LoadDir(filepath.Join(t.TempDir(), "does-not-exist"))
	if manifests != nil || errs != nil {
		t.Fatalf("missing dir should be empty: %v %v", manifests, errs)
	}
}

// A plugin's manifest is not allowed to grant itself the provider slot for a
// capability it doesn't own. Whoever answers a method sees every caller's
// params and returns whatever it likes, so `provides: ["*"]` or a bare core
// method would make installing a webview-only plugin — the configuration the
// manifest docs call fully confined — a way to sit in the middle of fs.read,
// agents.spawn and claude.approve.
func TestValidateProvidesConfinedToOwnNamespace(t *testing.T) {
	ok := []string{"acme.refresh", "acme.*"}
	for _, p := range ok {
		m := Manifest{ID: "acme", APIVersion: "1", Provides: []string{p}}
		if err := m.Validate(); err != nil {
			t.Errorf("provides %q should be allowed: %v", p, err)
		}
	}

	bad := []string{
		"*",              // everything
		"fs.read",        // a core capability
		"agents.spawn",   // ditto
		"claude.approve", // ditto
		"other.thing",    // another plugin's namespace
		"acme*",          // would also match "acmeother.secret"
		"acme.*.inner",   // wildcard in the middle
		"",               // empty
		"   ",            // whitespace only
	}
	for _, p := range bad {
		m := Manifest{ID: "acme", APIVersion: "1", Provides: []string{p}}
		if err := m.Validate(); err == nil {
			t.Errorf("provides %q should be rejected", p)
		}
	}
}

// A plugin already on disk from before the rule must lose only the grant it
// should never have had — not its ability to load.
func TestEventGrantsDropUnownedProvides(t *testing.T) {
	g := eventGrantsFor(Manifest{
		ID:       "acme",
		Provides: []string{"acme.refresh", "*", "fs.read"},
	})
	if len(g.Provides) != 1 || g.Provides[0] != "acme.refresh" {
		t.Fatalf("expected only the own-namespace grant to survive, got %v", g.Provides)
	}
}

// A project-scoped setting is stored in config.yaml, and `config.get` hands
// that whole document to any caller holding the capability *because* the config
// holds no credential (capspec.go says so in as many words). One secret project
// setting would make that reasoning false for every plugin at once, so the
// combination is refused at load rather than left to plugin authors to know.
func TestProjectScopedSettingCannotBeSecret(t *testing.T) {
	base := func(s SettingDef) *Manifest {
		return &Manifest{ID: "a.b", Name: "B", APIVersion: APIVersion, Settings: []SettingDef{s}}
	}

	err := base(SettingDef{Key: "repo", Label: "Repo", Type: "string", Scope: ScopeProject, Secret: true}).Validate()
	if err == nil {
		t.Fatal("a secret project-scoped setting must be refused")
	}
	if !strings.Contains(err.Error(), "config.yaml") {
		t.Errorf("the error must say WHY (config.yaml holds no credentials), got: %v", err)
	}

	// The two halves are each fine on their own — a global secret is the normal
	// way to hold a token, and a non-secret project setting is the whole point.
	if err := base(SettingDef{Key: "token", Label: "Token", Type: "string", Secret: true}).Validate(); err != nil {
		t.Errorf("a global secret is legitimate: %v", err)
	}
	if err := base(SettingDef{Key: "repo", Label: "Repo", Type: "string", Scope: ScopeProject}).Validate(); err != nil {
		t.Errorf("a non-secret project setting is the point: %v", err)
	}
	// Absent scope keeps meaning global, so every existing manifest still loads.
	if err := base(SettingDef{Key: "x", Label: "X", Type: "string"}).Validate(); err != nil {
		t.Errorf("an unscoped setting must still be valid: %v", err)
	}
	if err := base(SettingDef{Key: "x", Label: "X", Type: "string", Scope: "per-user"}).Validate(); err == nil {
		t.Error("an unknown scope must be refused rather than silently treated as global")
	}
}

// Contributed facade tools: presentation bound to a provides method. The
// validation rules exist so a manifest can't (a) declare a tool for a method
// it can't answer, (b) ship a name/schema the MCP layer would refuse (the SDK
// PANICS on a non-object schema), or (c) leave the model an undescribed tool.
func TestValidateTools(t *testing.T) {
	base := func(tool ToolDef) *Manifest {
		return &Manifest{
			ID:         "acme",
			APIVersion: APIVersion,
			Provides:   []string{"acme.search"},
			Tools:      []ToolDef{tool},
		}
	}

	ok := []ToolDef{
		{Name: "search", Description: "Search things.", Method: "acme.search"},
		{Name: "schema_d", Description: "S.", Method: "acme.search",
			InputSchema: json.RawMessage(`{"type":"object","properties":{"q":{"type":"string"}}}`)},
	}
	for _, tool := range ok {
		if err := base(tool).Validate(); err != nil {
			t.Errorf("tool %+v should be allowed: %v", tool, err)
		}
	}

	// A namespace wildcard (`acme.*` — the only wildcard validateProvides
	// permits) covers any method under it.
	wild := &Manifest{ID: "acme", APIVersion: APIVersion, Provides: []string{"acme.*"},
		Tools: []ToolDef{{Name: "sub_list", Description: "List.", Method: "acme.sub.list"}}}
	if err := wild.Validate(); err != nil {
		t.Errorf("wildcard-covered tool should be allowed: %v", err)
	}

	bad := []ToolDef{
		{Name: "search", Description: "D.", Method: "acme.other"},                                                    // no provides cover
		{Name: "search", Description: "D.", Method: "other.search"},                                                  // foreign namespace
		{Name: "search", Description: "", Method: "acme.search"},                                                     // no description
		{Name: "search", Description: "D.", Method: ""},                                                              // no method
		{Name: "Search", Description: "D.", Method: "acme.search"},                                                   // uppercase name
		{Name: "9lives", Description: "D.", Method: "acme.search"},                                                   // leading digit
		{Name: "has-dash", Description: "D.", Method: "acme.search"},                                                 // dash
		{Name: "search", Description: "D.", Method: "acme.search", InputSchema: json.RawMessage(`"str"`)},            // non-object schema
		{Name: "search", Description: "D.", Method: "acme.search", InputSchema: json.RawMessage(`{"type":"array"}`)}, // wrong type
	}
	for _, tool := range bad {
		if err := base(tool).Validate(); err == nil {
			t.Errorf("tool %+v should be rejected", tool)
		}
	}

	// Duplicate tool names are refused.
	m := &Manifest{ID: "acme", APIVersion: APIVersion, Provides: []string{"acme.*"},
		Tools: []ToolDef{
			{Name: "x", Description: "D.", Method: "acme.a"},
			{Name: "x", Description: "D.", Method: "acme.b"},
		}}
	if err := m.Validate(); err == nil {
		t.Error("duplicate tool names should be rejected")
	}
}

// ConsentedTools narrows to the PIN, not the manifest: a tool whose method
// rides a provides pattern added after consent is withheld from the facade
// until reinstall/reload re-baselines the pin.
func TestConsentedToolsNarrowedByGrantPin(t *testing.T) {
	dir := t.TempDir()
	pluginDir := filepath.Join(dir, "acme")
	if err := os.MkdirAll(pluginDir, 0o755); err != nil {
		t.Fatal(err)
	}
	mf := Manifest{
		ID:         "acme",
		APIVersion: APIVersion,
		Dir:        pluginDir,
		Provides:   []string{"acme.search"},
		Tools: []ToolDef{
			{Name: "search", Description: "D.", Method: "acme.search"},
		},
	}
	// Pin consents to the manifest as-is.
	ensureGrantPin(mf)

	mgr := NewManager(newCapture(), nil)
	mgr.AddAll([]Manifest{mf})
	defer mgr.Stop()

	got := mgr.ConsentedTools()
	if len(got) != 1 || len(got[0].Tools) != 1 || got[0].Tools[0].Name != "search" {
		t.Fatalf("expected the consented tool, got %+v", got)
	}

	// The plugin later self-amends provides + tools (plugin.json is inside its
	// own write root). The new tool's method is NOT in the pin → withheld.
	mf2 := mf
	mf2.Provides = []string{"acme.search", "acme.escalate"}
	mf2.Tools = append(mf2.Tools, ToolDef{Name: "escalate", Description: "D.", Method: "acme.escalate"})
	mgr2 := NewManager(newCapture(), nil)
	mgr2.AddAll([]Manifest{mf2})
	defer mgr2.Stop()

	got2 := mgr2.ConsentedTools()
	if len(got2) != 1 || len(got2[0].Tools) != 1 || got2[0].Tools[0].Name != "search" {
		t.Fatalf("post-consent tool must be withheld, got %+v", got2)
	}

	// Disabled plugins contribute nothing.
	mf3 := mf
	mf3.Disabled = true
	mgr3 := NewManager(newCapture(), nil)
	mgr3.AddAll([]Manifest{mf3})
	defer mgr3.Stop()
	if got3 := mgr3.ConsentedTools(); len(got3) != 0 {
		t.Fatalf("disabled plugin must contribute no tools, got %+v", got3)
	}
}
