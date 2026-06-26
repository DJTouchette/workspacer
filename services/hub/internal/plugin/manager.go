package plugin

import (
	"crypto/rand"
	"encoding/base64"
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

// TokenRegistrar lets the manager bind a per-plugin bus token to the plugin's
// declared capabilities, so the bus can scope what each plugin may call. The bus
// Server implements this. nil is allowed (capability enforcement disabled).
type TokenRegistrar interface {
	RegisterPluginToken(token, pluginID string, caps []string)
	UnregisterPluginToken(token string)
}

// Manager owns the loaded plugins: it supervises their sidecars and announces
// their contributions on the bus (plugin.loaded / plugin.unloaded), so the UI
// can register the pane types and hotkeys they inject.
type Manager struct {
	pub Publisher
	reg TokenRegistrar // may be nil (capability enforcement off)

	mu      sync.Mutex
	plugins map[string]*loaded
}

type loaded struct {
	manifest Manifest
	sup      *supervisor.Supervisor // nil for metadata-only plugins
	token    string                 // per-plugin bus token ("" if no sidecar/registrar)
}

// NewManager creates a manager that publishes lifecycle events to pub and binds
// per-plugin bus tokens via reg (nil to disable capability enforcement).
func NewManager(pub Publisher, reg TokenRegistrar) *Manager {
	return &Manager{pub: pub, reg: reg, plugins: make(map[string]*loaded)}
}

// loadOrCreatePluginToken returns the plugin's persisted bus token, minting and
// writing one on first use. Stable across restarts so webview pane URLs stay
// valid. Best-effort: a write failure still returns a usable in-memory token.
func loadOrCreatePluginToken(dir string) string {
	file := filepath.Join(dir, busTokenFile)
	if existing := readSidecar(file); existing != "" {
		return existing
	}
	buf := make([]byte, 24)
	if _, err := rand.Read(buf); err != nil {
		return ""
	}
	token := base64.RawURLEncoding.EncodeToString(buf)
	_ = os.WriteFile(file, []byte(token), 0o600)
	return token
}

// Tokens returns the per-plugin bus tokens keyed by plugin id, for the host to
// inject into each plugin's webview URL. Only the trusted host can read these
// (served on a token-guarded route).
func (m *Manager) Tokens() map[string]string {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make(map[string]string, len(m.plugins))
	for id, l := range m.plugins {
		if l.token != "" {
			out[id] = l.token
		}
	}
	return out
}

// Add registers a plugin: starts its sidecar (if any) and emits plugin.loaded.
func (m *Manager) Add(mf Manifest) {
	l := &loaded{manifest: mf}
	if mf.Server != nil && !mf.Disabled {
		// Mint/persist this plugin's bus token and bind it to its declared
		// capabilities, then hand it to the sidecar via env (HUB_TOKEN). The
		// sidecar — and the plugin's webview, which gets the same token injected
		// into its pane URL by the host — connects with it and is scoped to caps.
		var env []string
		if m.reg != nil {
			l.token = loadOrCreatePluginToken(mf.Dir)
			if l.token != "" {
				m.reg.RegisterPluginToken(l.token, mf.ID, mf.Capabilities)
				env = []string{"HUB_TOKEN=" + l.token}
			}
		}
		l.sup = supervisor.New(supervisor.Spec{
			Name:      mf.ID,
			Command:   mf.Server.Command,
			Args:      mf.Server.Args,
			Dir:       mf.Dir, // run the sidecar in its own plugin directory
			Env:       env,
			HealthURL: healthURL(mf.Server),
		}, m.pub)
		l.sup.Start()
	}

	m.mu.Lock()
	prev, hadPrev := m.plugins[mf.ID]
	m.plugins[mf.ID] = l
	m.mu.Unlock()
	if hadPrev {
		// Stop the previous supervisor (avoid a goroutine leak) and drop its token
		// unless we reused the same one (a stable reload keeps the persisted token).
		if prev.sup != nil {
			prev.sup.Stop()
		}
		if m.reg != nil && prev.token != "" && prev.token != l.token {
			m.reg.UnregisterPluginToken(prev.token)
		}
	}

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
	if m.reg != nil && l.token != "" {
		m.reg.UnregisterPluginToken(l.token)
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
