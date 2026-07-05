package plugin

import (
	"crypto/rand"
	"encoding/base64"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/sandbox"
	"github.com/djtouchette/workspacer-hub/internal/supervisor"
)

// grantsFor translates a manifest's declared capabilities into bus grants,
// resolving each path-scoped capability's roots against the plugin's own
// directory. This is the static, load-time grant: dynamic scopes like
// ${agentCwd} aren't bound here and so resolve to nothing — the static
// per-plugin token gets no filesystem reach for them. PaneToken binds those
// per open pane.
func grantsFor(mf Manifest) []capspec.Grant {
	return grantsWithBindings(mf, nil)
}

// grantsWithBindings resolves a manifest's capabilities into grants using the
// plugin's directory plus any extra bindings the caller supplies (e.g.
// {"agentCwd": "/path"} when minting a token for an agent-scoped pane).
func grantsWithBindings(mf Manifest, extra map[string]string) []capspec.Grant {
	bindings := map[string]string{"pluginDir": mf.Dir}
	for k, v := range extra {
		bindings[k] = v
	}
	out := make([]capspec.Grant, 0, len(mf.Capabilities))
	for _, c := range mf.Capabilities {
		if c.Method == "" {
			continue
		}
		out = append(out, capspec.Grant{Method: c.Method, FSRoots: resolveRoots(c.Paths, bindings)})
	}
	return out
}

// eventGrantsFor lifts a manifest's declared pub/sub + provider surface into the
// grant the bus enforces: which event types the plugin may publish (emits) /
// receive (consumes), and which capability methods it may register as a provider
// of (provides). Verbatim from the manifest — the patterns are matched at the
// bus with the same syntax as subscription topics.
func eventGrantsFor(mf Manifest) capspec.EventGrants {
	return capspec.EventGrants{
		Emits:    mf.Emits,
		Consumes: mf.Consumes,
		Provides: mf.Provides,
	}
}

// resolveRoots expands a capability's declared path scopes to concrete roots.
// Unresolvable entries are dropped — a path-scoped grant that resolves to no
// roots denies every call (fail closed), which is the safe outcome.
func resolveRoots(paths []string, bindings map[string]string) []string {
	if len(paths) == 0 {
		return nil
	}
	out := make([]string, 0, len(paths))
	for _, p := range paths {
		if r := expandScope(p, bindings); r != "" {
			out = append(out, r)
		}
	}
	return out
}

// expandScope resolves one path-scope entry against the available bindings.
// A "${name}" or "${name}/sub" token expands to the binding's value (joined with
// the subpath); an absolute path passes through. Anything whose binding is
// missing/empty — including a relative path or an as-yet-unbound token — yields
// "" so it grants nothing.
func expandScope(p string, bindings map[string]string) string {
	if strings.HasPrefix(p, "${") {
		end := strings.Index(p, "}")
		if end < 0 {
			return ""
		}
		base, ok := bindings[p[2:end]]
		if !ok || base == "" {
			return ""
		}
		rest := strings.TrimPrefix(p[end+1:], "/")
		if rest == "" {
			return base
		}
		return filepath.Join(base, filepath.FromSlash(rest))
	}
	if filepath.IsAbs(p) {
		return p
	}
	return ""
}

// Publisher is the slice of the broker the manager needs.
type Publisher interface {
	Publish(event.Envelope)
}

// TokenRegistrar lets the manager bind a per-plugin bus token to the plugin's
// declared capability grants, so the bus can scope what each plugin may call and
// confine its filesystem reach. The bus Server implements this. nil is allowed
// (capability enforcement disabled).
type TokenRegistrar interface {
	RegisterPluginToken(token, pluginID string, grants []capspec.Grant, events capspec.EventGrants)
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
	// Ephemeral per-pane tokens (token → plugin id), minted by PaneToken with
	// dynamic scopes resolved (e.g. ${agentCwd}). Tracked so they can be revoked
	// on pane close and swept when their plugin is removed/stopped.
	paneTokens map[string]string

	// How sidecars are launched under OS-level filesystem confinement. Default
	// best-effort (confine when the platform supports it, else run plain). Set by
	// the hub from WORKSPACER_PLUGIN_SANDBOX.
	sandboxMode sandbox.Mode

	// Serializes the read-modify-write of a plugin's persisted settings overlay
	// (settings.go) so concurrent SetSettings calls can't lose an update or read a
	// torn file. Separate from mu so settings IO never blocks pane-token / lifecycle
	// operations.
	settingsMu sync.Mutex
}

type loaded struct {
	manifest Manifest
	sup      *supervisor.Supervisor // nil for metadata-only plugins
	token    string                 // per-plugin bus token ("" if no sidecar/registrar)
}

// NewManager creates a manager that publishes lifecycle events to pub and binds
// per-plugin bus tokens via reg (nil to disable capability enforcement).
func NewManager(pub Publisher, reg TokenRegistrar) *Manager {
	return &Manager{
		pub:         pub,
		reg:         reg,
		plugins:     make(map[string]*loaded),
		paneTokens:  make(map[string]string),
		sandboxMode: sandbox.ModeBestEffort,
	}
}

// SetSandboxMode sets how sidecars are launched under filesystem confinement
// (off / best-effort / enforce). Call before loading plugins.
func (m *Manager) SetSandboxMode(mode sandbox.Mode) {
	m.mu.Lock()
	m.sandboxMode = mode
	m.mu.Unlock()
}

// sandboxSidecar resolves how to launch mf's sidecar under the current mode,
// emits a lifecycle event, and reports whether it should start at all (false
// means refused — enforce mode on a platform with no confinement mechanism).
func (m *Manager) sandboxSidecar(mf Manifest) (command string, args []string, run bool) {
	m.mu.Lock()
	mode := m.sandboxMode
	m.mu.Unlock()
	// Expand ${os}/${arch}/${exe} so a manifest can name a prebuilt per-platform
	// binary (e.g. ./bin/${os}-${arch}/server${exe}) instead of a single command
	// that only runs on the OS it was committed from.
	cmd := expandPlatformTokens(mf.Server.Command)
	cmdArgs := expandPlatformTokensAll(mf.Server.Args)
	// A sidecar may write only its own plugin directory (a private temp is added
	// by the mechanism). Reads stay open so it can load its interpreter/libraries.
	res := sandbox.Wrap(cmd, cmdArgs, sandbox.Policy{WriteRoots: []string{mf.Dir}})
	switch sandbox.Decide(mode, res.Available) {
	case sandbox.RunSandboxed:
		m.pub.Publish(event.New("plugin.sandboxed", "hub", map[string]string{"id": mf.ID, "mechanism": res.Mechanism}))
		return res.Path, res.Args, true
	case sandbox.Refuse:
		m.pub.Publish(event.New("plugin.sandbox.refused", "hub", map[string]string{"id": mf.ID, "reason": res.Note}))
		return "", nil, false
	default: // RunUnsandboxed
		if mode != sandbox.ModeOff {
			// best-effort + no available mechanism: the sidecar is ACTUALLY running
			// unconfined and nothing else makes that visible. Warn loudly so the
			// operator sees the risk in the hub output, not just on the bus.
			log.Printf("[plugin] WARNING: sidecar %q is running WITHOUT sandboxing (no confinement mechanism available on this platform / mode=best-effort: %s) — it has full access to your files and network. Set WORKSPACER_PLUGIN_SANDBOX=enforce to refuse unconfined plugins.", mf.ID, res.Note)
			m.pub.Publish(event.New("plugin.unsandboxed", "hub", map[string]string{"id": mf.ID, "reason": res.Note}))
		} else {
			// mode=off: the operator explicitly turned sandboxing off — one quiet note.
			log.Printf("[plugin] sidecar %q running unsandboxed (WORKSPACER_PLUGIN_SANDBOX=off)", mf.ID)
		}
		return cmd, cmdArgs, true
	}
}

// randomToken mints a fresh URL-safe bus token.
func randomToken() (string, error) {
	buf := make([]byte, 24)
	if _, err := rand.Read(buf); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(buf), nil
}

// PaneToken mints an ephemeral, capability-scoped bus token for one open pane of
// a plugin, resolving the plugin's dynamic path scopes (e.g. ${agentCwd}) with
// bindings the trusted host supplies for that pane. The host injects the
// returned token into the pane's webview URL; the webview then has exactly the
// plugin's capabilities, confined to this pane's resolved roots. Revoke it with
// RevokePaneToken when the pane closes (it is also swept if the plugin is
// removed or the manager stops). Requires capability enforcement (reg != nil).
func (m *Manager) PaneToken(pluginID string, bindings map[string]string) (string, error) {
	if m.reg == nil {
		return "", fmt.Errorf("capability enforcement is off; pane tokens unavailable")
	}
	tok, err := randomToken()
	if err != nil {
		return "", err
	}
	// Register the token on the bus and record it here under one hold of m.mu, so
	// a concurrent Remove (which sweeps paneTokens via revokePaneTokensFor) can't
	// interleave between the two steps. The old two-step version registered
	// outside the lock: a Remove that ran after RegisterPluginToken but before the
	// paneTokens insert would fail to see the token and leave it registered on the
	// bus but untracked — an unrevocable grant leak. Re-checking m.plugins under
	// the same lock also means a token is never minted for an already-removed
	// plugin. (RegisterPluginToken canonicalizes roots under the lock; the manager
	// lock isn't hot, so the extra hold is acceptable for the atomicity it buys.)
	m.mu.Lock()
	defer m.mu.Unlock()
	l, ok := m.plugins[pluginID]
	if !ok {
		return "", fmt.Errorf("plugin %q is not loaded", pluginID)
	}
	m.reg.RegisterPluginToken(tok, pluginID, grantsWithBindings(l.manifest, bindings), eventGrantsFor(l.manifest))
	m.paneTokens[tok] = pluginID
	return tok, nil
}

// RevokePaneToken drops an ephemeral pane token (on pane close). Safe to call
// with an unknown or empty token.
func (m *Manager) RevokePaneToken(token string) {
	if token == "" {
		return
	}
	m.mu.Lock()
	_, ok := m.paneTokens[token]
	delete(m.paneTokens, token)
	m.mu.Unlock()
	if ok && m.reg != nil {
		m.reg.UnregisterPluginToken(token)
	}
}

// revokePaneTokensFor sweeps every outstanding pane token of a plugin (called
// when it's removed/stopped, so a closed plugin's panes can't keep calling).
func (m *Manager) revokePaneTokensFor(pluginID string) {
	m.mu.Lock()
	var gone []string
	for tok, id := range m.paneTokens {
		if id == pluginID {
			gone = append(gone, tok)
			delete(m.paneTokens, tok)
		}
	}
	m.mu.Unlock()
	if m.reg != nil {
		for _, tok := range gone {
			m.reg.UnregisterPluginToken(tok)
		}
	}
}

// loadOrCreatePluginToken returns the plugin's persisted bus token, minting and
// writing one on first use. Stable across restarts so webview pane URLs stay
// valid. Best-effort: a write failure still returns a usable in-memory token.
func loadOrCreatePluginToken(dir string) string {
	file := filepath.Join(dir, busTokenFile)
	if existing := readSidecar(file); existing != "" {
		return existing
	}
	token, err := randomToken()
	if err != nil {
		return ""
	}
	// Persist so the token is stable across restarts — webview pane and share URLs
	// embed it, and a fresh token silently invalidates every saved URL. Still
	// best-effort (we return a usable in-memory token on failure), but a swallowed
	// write error is exactly how that silent invalidation would go unnoticed, so
	// log it loudly with the path and likely cause.
	if err := os.WriteFile(file, []byte(token), 0o600); err != nil {
		log.Printf("[plugin] ERROR: could not persist bus token to %s: %v — this token is in-memory only, so restarting the hub will mint a new one and invalidate any saved webview/share URLs for this plugin. Check the plugin directory's permissions and free disk space.", file, err)
	}
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

// UIDir returns the absolute directory of a webview-only plugin's hub-served
// static assets, or ok=false if the plugin isn't loaded or declares no ui. The
// hub's /plugins/ui route reads from here.
func (m *Manager) UIDir(id string) (string, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	l, ok := m.plugins[id]
	if !ok || l.manifest.UI == "" {
		return "", false
	}
	return filepath.Join(l.manifest.Dir, filepath.FromSlash(l.manifest.UI)), true
}

// Add registers a plugin: starts its sidecar (if any) and emits plugin.loaded.
func (m *Manager) Add(mf Manifest) {
	l := &loaded{manifest: mf}
	// Mint/persist a bus token and bind it to the plugin's declared capabilities
	// whenever something will connect to the bus as this plugin — a sidecar
	// (HUB_TOKEN env) or a webview pane (token injected into its URL by the host).
	// A webview-only plugin has no sidecar but still needs a token for its pane.
	if !mf.Disabled && m.reg != nil && (mf.Server != nil || len(mf.Panes) > 0) {
		l.token = loadOrCreatePluginToken(mf.Dir)
		if l.token != "" {
			m.reg.RegisterPluginToken(l.token, mf.ID, grantsFor(mf), eventGrantsFor(mf))
		}
	}
	if mf.Server != nil && !mf.Disabled {
		var env []string
		if l.token != "" {
			env = []string{"HUB_TOKEN=" + l.token}
		}
		// Launch under OS filesystem confinement (bwrap / sandbox-exec). In
		// enforce mode on a platform without a mechanism, run is false and the
		// sidecar is not started.
		cmd, cmdArgs, run := m.sandboxSidecar(mf)
		if run {
			l.sup = supervisor.New(supervisor.Spec{
				Name:      mf.ID,
				Command:   cmd,
				Args:      cmdArgs,
				Dir:       mf.Dir, // run the sidecar in its own plugin directory
				Env:       env,
				HealthURL: healthURL(mf.Server),
			}, m.pub)
			l.sup.Start()
		}
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
	m.revokePaneTokensFor(id) // drop any open-pane tokens for this plugin
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
