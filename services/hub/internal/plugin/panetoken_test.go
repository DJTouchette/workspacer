package plugin

import (
	"sync"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// fakeRegistrar records token registrations the way the bus would, so manager
// tests can assert what grants a pane token was bound to without a live bus.
type fakeRegistrar struct {
	mu         sync.Mutex
	registered map[string][]capspec.Grant
}

func newFakeRegistrar() *fakeRegistrar {
	return &fakeRegistrar{registered: map[string][]capspec.Grant{}}
}

func (f *fakeRegistrar) RegisterPluginToken(token, _ string, grants []capspec.Grant) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.registered[token] = grants
}

func (f *fakeRegistrar) UnregisterPluginToken(token string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	delete(f.registered, token)
}

func (f *fakeRegistrar) grants(token string) ([]capspec.Grant, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	g, ok := f.registered[token]
	return g, ok
}

func (f *fakeRegistrar) count() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.registered)
}

// rootsOf finds the fs roots a token's grant for method declares.
func rootsOf(grants []capspec.Grant, method string) []string {
	for _, g := range grants {
		if g.Method == method {
			return g.FSRoots
		}
	}
	return nil
}

func loadedManager(t *testing.T, reg TokenRegistrar, mf Manifest) *Manager {
	t.Helper()
	m := NewManager(&capture{ch: make(chan event.Envelope, 16)}, reg)
	// Register directly into the plugins map (no sidecar to supervise here).
	m.mu.Lock()
	m.plugins[mf.ID] = &loaded{manifest: mf}
	m.mu.Unlock()
	return m
}

func TestPaneTokenResolvesAgentCwd(t *testing.T) {
	reg := newFakeRegistrar()
	mf := Manifest{
		ID:  "acme.editor",
		Dir: "/plugins/acme",
		Capabilities: []Capability{
			{Method: "fs.read", Paths: []string{"${agentCwd}"}},
			{Method: "agents.list"},
		},
	}
	m := loadedManager(t, reg, mf)

	tok, err := m.PaneToken("acme.editor", map[string]string{"agentCwd": "/work/project"})
	if err != nil {
		t.Fatal(err)
	}
	grants, ok := reg.grants(tok)
	if !ok {
		t.Fatal("pane token was not registered with the bus")
	}
	// fs.read should now be scoped to the resolved agent cwd.
	roots := rootsOf(grants, "fs.read")
	if len(roots) != 1 || roots[0] != "/work/project" {
		t.Fatalf("fs.read roots = %v, want [/work/project]", roots)
	}
	// The verb-only capability is still present, with no roots.
	if r := rootsOf(grants, "agents.list"); len(r) != 0 {
		t.Fatalf("agents.list should have no roots, got %v", r)
	}
}

func TestStaticTokenHasNoAgentCwdReach(t *testing.T) {
	// At static load, ${agentCwd} is unbound → fs.read resolves to no roots, so
	// the persistent per-plugin token can't touch the filesystem at all.
	mf := Manifest{
		ID:           "acme.editor",
		Dir:          "/plugins/acme",
		Capabilities: []Capability{{Method: "fs.read", Paths: []string{"${agentCwd}"}}},
	}
	grants := grantsFor(mf)
	if r := rootsOf(grants, "fs.read"); len(r) != 0 {
		t.Fatalf("static fs.read roots = %v, want none (agentCwd unbound)", r)
	}
}

func TestRevokePaneToken(t *testing.T) {
	reg := newFakeRegistrar()
	mf := Manifest{ID: "acme.editor", Dir: "/plugins/acme",
		Capabilities: []Capability{{Method: "fs.read", Paths: []string{"${agentCwd}"}}}}
	m := loadedManager(t, reg, mf)

	tok, err := m.PaneToken("acme.editor", map[string]string{"agentCwd": "/work/p"})
	if err != nil {
		t.Fatal(err)
	}
	if reg.count() != 1 {
		t.Fatalf("expected 1 registered token, got %d", reg.count())
	}
	m.RevokePaneToken(tok)
	if reg.count() != 0 {
		t.Fatalf("expected token revoked, %d still registered", reg.count())
	}
	// Revoking again / an unknown token is a no-op.
	m.RevokePaneToken(tok)
	m.RevokePaneToken("nonexistent")
}

func TestRemoveSweepsPaneTokens(t *testing.T) {
	reg := newFakeRegistrar()
	mf := Manifest{ID: "acme.editor", Dir: "/plugins/acme",
		Capabilities: []Capability{{Method: "fs.read", Paths: []string{"${agentCwd}"}}}}
	m := loadedManager(t, reg, mf)

	if _, err := m.PaneToken("acme.editor", map[string]string{"agentCwd": "/a"}); err != nil {
		t.Fatal(err)
	}
	if _, err := m.PaneToken("acme.editor", map[string]string{"agentCwd": "/b"}); err != nil {
		t.Fatal(err)
	}
	if reg.count() != 2 {
		t.Fatalf("expected 2 pane tokens, got %d", reg.count())
	}
	m.Remove("acme.editor")
	if reg.count() != 0 {
		t.Fatalf("removing the plugin should sweep its pane tokens, %d left", reg.count())
	}
}

func TestPaneTokenUnknownPluginAndNoEnforcement(t *testing.T) {
	reg := newFakeRegistrar()
	m := loadedManager(t, reg, Manifest{ID: "acme.editor", Dir: "/plugins/acme"})
	if _, err := m.PaneToken("nope", nil); err == nil {
		t.Fatal("expected error for unknown plugin")
	}

	// With enforcement off (nil registrar) pane tokens are unavailable.
	off := loadedManager(t, nil, Manifest{ID: "acme.editor", Dir: "/plugins/acme"})
	if _, err := off.PaneToken("acme.editor", nil); err == nil {
		t.Fatal("expected error when capability enforcement is off")
	}
}
