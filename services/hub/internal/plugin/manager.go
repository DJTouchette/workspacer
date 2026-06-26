package plugin

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/supervisor"
)

// Publisher is the slice of the broker the manager needs.
type Publisher interface {
	Publish(event.Envelope)
}

// Manager owns the loaded plugins: it supervises their sidecars and announces
// their contributions on the bus (plugin.loaded / plugin.unloaded), so the UI
// can register the pane types and hotkeys they inject.
type Manager struct {
	pub Publisher

	mu      sync.Mutex
	plugins map[string]*loaded
}

type loaded struct {
	manifest Manifest
	sup      *supervisor.Supervisor // nil for metadata-only plugins
}

// NewManager creates a manager that publishes lifecycle events to pub.
func NewManager(pub Publisher) *Manager {
	return &Manager{pub: pub, plugins: make(map[string]*loaded)}
}

// Add registers a plugin: starts its sidecar (if any) and emits plugin.loaded.
func (m *Manager) Add(mf Manifest) {
	l := &loaded{manifest: mf}
	if mf.Server != nil && !mf.Disabled {
		l.sup = supervisor.New(supervisor.Spec{
			Name:      mf.ID,
			Command:   mf.Server.Command,
			Args:      mf.Server.Args,
			Dir:       mf.Dir, // run the sidecar in its own plugin directory
			HealthURL: healthURL(mf.Server),
		}, m.pub)
		l.sup.Start()
	}

	m.mu.Lock()
	if prev, ok := m.plugins[mf.ID]; ok && prev.sup != nil {
		// Stop the previous supervisor before replacing it to avoid a goroutine
		// leak. Unlock first so Stop can acquire the mutex if needed.
		m.mu.Unlock()
		prev.sup.Stop()
		m.mu.Lock()
	}
	m.plugins[mf.ID] = l
	m.mu.Unlock()

	m.pub.Publish(event.New("plugin.loaded", "hub", mf))
}

// AddAll registers a batch of plugins.
func (m *Manager) AddAll(manifests []Manifest) {
	for _, mf := range manifests {
		m.Add(mf)
	}
}

// Remove stops a plugin's sidecar and emits plugin.unloaded.
// It returns the plugin's directory path (empty string if the id was not
// found) so the caller can delete the directory without an additional
// List() call — avoiding the TOCTOU window that existed when the handler
// called List() and Remove() in two separate unlocked steps.
func (m *Manager) Remove(id string) string {
	m.mu.Lock()
	l, ok := m.plugins[id]
	delete(m.plugins, id)
	m.mu.Unlock()
	if !ok {
		return ""
	}
	if l.sup != nil {
		l.sup.Stop()
	}
	m.pub.Publish(event.New("plugin.unloaded", "hub", map[string]string{"id": id}))
	return l.manifest.Dir
}

// SetEnabled toggles a plugin's disabled marker (<dir>/.disabled), then reloads
// it so its sidecar is started or stopped and a plugin.loaded event refreshes
// the UI. Returns the reloaded manifest. Disabling stops a running sidecar;
// enabling starts it (if the plugin declares a server).
func (m *Manager) SetEnabled(id string, enabled bool) (Manifest, error) {
	m.mu.Lock()
	l, ok := m.plugins[id]
	dir := ""
	if ok {
		dir = l.manifest.Dir
	}
	m.mu.Unlock()
	if !ok {
		return Manifest{}, fmt.Errorf("plugin %q not found", id)
	}
	if dir == "" {
		return Manifest{}, fmt.Errorf("plugin %q has no directory on disk", id)
	}

	marker := filepath.Join(dir, disabledFile)
	if enabled {
		if err := os.Remove(marker); err != nil && !os.IsNotExist(err) {
			return Manifest{}, err
		}
	} else if err := os.WriteFile(marker, []byte("disabled\n"), 0o644); err != nil {
		return Manifest{}, err
	}

	// Re-load so Disabled reflects the marker we just changed, then re-add:
	// Add stops the previous sidecar and starts a new one only when enabled.
	mf, err := Load(filepath.Join(dir, "plugin.json"))
	if err != nil {
		return Manifest{}, err
	}
	m.Add(mf)
	return mf, nil
}

// Stop tears every plugin down (used on hub shutdown).
func (m *Manager) Stop() {
	m.mu.Lock()
	ids := make([]string, 0, len(m.plugins))
	for id := range m.plugins {
		ids = append(ids, id)
	}
	m.mu.Unlock()
	for _, id := range ids {
		m.Remove(id)
	}
}

// List returns the manifests of all loaded plugins (for the /plugins endpoint).
func (m *Manager) List() []Manifest {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]Manifest, 0, len(m.plugins))
	for _, l := range m.plugins {
		out = append(out, l.manifest)
	}
	return out
}

// State reports a plugin's supervisor state ("" if unknown / metadata-only).
func (m *Manager) State(id string) supervisor.State {
	m.mu.Lock()
	defer m.mu.Unlock()
	if l, ok := m.plugins[id]; ok && l.sup != nil {
		return l.sup.State()
	}
	return ""
}

func healthURL(s *ServerSpec) string {
	if s == nil || s.Health == "" || s.Port == 0 {
		return ""
	}
	return fmt.Sprintf("http://127.0.0.1:%d%s", s.Port, s.Health)
}
