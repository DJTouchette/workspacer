package plugin

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// The bundled webview-only example must load and validate — it's the template
// for sidecar-less plugins (and the editor extraction), so a regression here
// breaks the canonical shape.
func TestExampleHelloScopedLoads(t *testing.T) {
	path := filepath.Join("..", "..", "examples", "hello-scoped", "plugin.json")
	mf, err := Load(path)
	if err != nil {
		t.Fatalf("loading the hello-scoped example: %v", err)
	}
	if mf.Server != nil {
		t.Error("the example should be webview-only (no server)")
	}
	if mf.UI == "" {
		t.Error("the example should declare a ui directory")
	}
	// The declared ui dir should actually exist next to the manifest.
	if _, statErr := os.Stat(filepath.Join(mf.Dir, mf.UI, "index.html")); statErr != nil {
		t.Errorf("example ui/index.html missing: %v", statErr)
	}
}

func TestValidate_PanesNeedServerOrUI(t *testing.T) {
	pane := []PaneContribution{{Type: "acme.view", Title: "View"}}

	// Panes with neither a server nor a ui dir → rejected.
	bad := Manifest{ID: "x", APIVersion: APIVersion, Panes: pane}
	if err := bad.Validate(); err == nil {
		t.Fatal("expected panes with no server and no ui to be rejected")
	}

	// Webview-only: panes + ui (no server) → fine.
	ui := Manifest{ID: "x", APIVersion: APIVersion, Panes: pane, UI: "ui"}
	if err := ui.Validate(); err != nil {
		t.Fatalf("webview-only plugin should validate, got %v", err)
	}

	// Sidecar: panes + server (no ui) → fine.
	side := Manifest{ID: "x", APIVersion: APIVersion, Panes: pane, Server: &ServerSpec{Command: "./srv"}}
	if err := side.Validate(); err != nil {
		t.Fatalf("sidecar plugin should validate, got %v", err)
	}

	// No panes at all → ui/server irrelevant.
	none := Manifest{ID: "x", APIVersion: APIVersion}
	if err := none.Validate(); err != nil {
		t.Fatalf("plugin with no panes should validate, got %v", err)
	}
}

func TestUIDir(t *testing.T) {
	reg := newFakeRegistrar()
	mf := Manifest{ID: "acme.editor", Dir: "/plugins/acme", UI: "dist",
		Panes: []PaneContribution{{Type: "acme.editor", Title: "Editor"}}}
	m := loadedManager(t, reg, mf)

	dir, ok := m.UIDir("acme.editor")
	if !ok || dir != filepath.FromSlash("/plugins/acme/dist") {
		t.Fatalf("UIDir = (%q, %v), want (/plugins/acme/dist, true)", dir, ok)
	}

	// A plugin with no ui, and an unknown plugin, both report false.
	noUI := loadedManager(t, reg, Manifest{ID: "p", Dir: "/p"})
	if _, ok := noUI.UIDir("p"); ok {
		t.Error("plugin without ui should report no UIDir")
	}
	if _, ok := m.UIDir("nope"); ok {
		t.Error("unknown plugin should report no UIDir")
	}
}

// A webview-only plugin (no sidecar) must still get a bus token registered, or
// its pane could never connect to the bus.
func TestAdd_WebviewOnlyRegistersToken(t *testing.T) {
	reg := newFakeRegistrar()
	m := NewManager(&capture{ch: make(chan event.Envelope, 16)}, reg)
	m.Add(Manifest{
		ID:           "acme.editor",
		APIVersion:   APIVersion,
		Dir:          t.TempDir(),
		UI:           "dist",
		Panes:        []PaneContribution{{Type: "acme.editor", Title: "Editor"}},
		Capabilities: []Capability{{Method: "agents.list"}},
	})
	if reg.count() != 1 {
		t.Fatalf("webview-only plugin should register a bus token, got %d", reg.count())
	}
	// And it should have no sidecar supervisor.
	if m.State("acme.editor") != "" {
		t.Fatalf("webview-only plugin should have no sidecar state, got %q", m.State("acme.editor"))
	}
}

// A metadata-only plugin (no server, no panes) registers no token — nothing
// connects as it.
func TestAdd_MetadataOnlyRegistersNoToken(t *testing.T) {
	reg := newFakeRegistrar()
	m := NewManager(&capture{ch: make(chan event.Envelope, 16)}, reg)
	m.Add(Manifest{ID: "meta", APIVersion: APIVersion, Dir: t.TempDir()})
	if reg.count() != 0 {
		t.Fatalf("metadata-only plugin should register no token, got %d", reg.count())
	}
}
