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
		{APIVersion: "1"},                                                  // no id
		{ID: "x", APIVersion: "0"},                                         // bad version
		{ID: "x", APIVersion: "1", Server: &ServerSpec{}},                  // server no command
		{ID: "x", APIVersion: "1", Panes: []PaneContribution{{Type: ""}}},  // empty pane type
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
