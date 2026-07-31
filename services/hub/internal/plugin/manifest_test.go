package plugin

import (
	"os"
	"path/filepath"
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

func TestLoadDir(t *testing.T) {
	root := t.TempDir()
	writePlugin(t, root, "tracker", `{
		"id":"acme.tracker","name":"Tracker","apiVersion":"1",
		"server":{"command":"tracker-bin","port":9100,"health":"/healthz"},
		"panes":[{"type":"acme.tracker","title":"Issues","icon":"📋","path":"/ui"}],
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
