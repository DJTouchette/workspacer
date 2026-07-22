// Package plugin loads plugin manifests and supervises their sidecar processes.
// A plugin is a polyglot sidecar (any language) plus a manifest declaring how to
// start it and what it contributes to workspacer: pane types, hotkeys, the
// capabilities it provides/needs, and the events it emits/consumes.
package plugin

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// APIVersion is the manifest schema version this hub understands.
const APIVersion = "1"

// Sidecar marker files the loader reads from a plugin's directory (not part of
// the author's plugin.json):
//   - sourceFile records the install reference, enabling one-click update.
//   - disabledFile, when present, marks the plugin disabled (sidecar not started,
//     contributions withheld) while keeping it installed.
const (
	sourceFile   = ".install-source"
	disabledFile = ".disabled"
	// busTokenFile holds the plugin's per-plugin bus token. Persisted so the
	// token is stable across hub restarts — webview pane URLs (which carry it)
	// stay valid across saved-layout restores.
	busTokenFile = ".bus-token"
)

// Manifest is a plugin's plugin.json.
type Manifest struct {
	ID         string `json:"id"`
	Name       string `json:"name"`
	APIVersion string `json:"apiVersion"`

	// Version is the author-declared release version of the plugin (semver-ish,
	// e.g. "1.4.0", optional leading "v"). It is what the update check compares:
	// the installed manifest's Version against the same field in the manifest
	// fetched fresh from the install Source. Optional — a plugin with no Version
	// can be reinstalled but never reports an update available (there is nothing
	// to compare), which is deliberately how we avoid a permanent false "update".
	Version string `json:"version,omitempty"`

	Server   *ServerSpec          `json:"server,omitempty"`
	Panes    []PaneContribution   `json:"panes,omitempty"`
	Hotkeys  []HotkeyContribution `json:"hotkeys,omitempty"`
	Settings []SettingDef         `json:"settings,omitempty"`

	// UI: a subdirectory (relative to the plugin dir) of static assets the hub
	// serves for this plugin's panes, at /plugins/ui/<id>/. Set it instead of
	// `server` for a *webview-only* plugin — one with no sidecar process. The
	// hub (trusted) serves the files; the webview talks to the bus with its
	// scoped token. With nothing arbitrary to run, there is nothing to escape
	// the bus through, so capability scoping fully confines it. Only the named
	// subdir is exposed — the plugin's manifest and .bus-token (in the dir root)
	// are not.
	UI string `json:"ui,omitempty"`

	// Provides: capabilities this plugin answers on the bus.
	// Capabilities: capabilities it may call (each optionally path-scoped).
	Provides     []string     `json:"provides,omitempty"`
	Capabilities []Capability `json:"capabilities,omitempty"`
	Emits        []string     `json:"emits,omitempty"`
	Consumes     []string     `json:"consumes,omitempty"`

	// Install: a one-time setup command (argv) run in the plugin dir after a
	// GitHub install — e.g. ["npm","install"] or ["go","build","-o","bin"].
	// Empty = nothing to build (self-contained plugin).
	Install []string `json:"install,omitempty"`

	// Dir is the directory the manifest was loaded from (set by the loader).
	Dir string `json:"-"`

	// Source is the install reference (GitHub URL / owner-repo) recorded at
	// install time, so the UI can offer one-click update. Populated by the loader
	// from <dir>/.install-source; empty for plugins dropped in by hand.
	Source string `json:"source,omitempty"`

	// Disabled reflects the presence of <dir>/.disabled: the plugin is installed
	// but its sidecar isn't started and its panes/hotkeys are withheld. Populated
	// by the loader.
	Disabled bool `json:"disabled,omitempty"`
}

// Capability is one entry of a manifest's "capabilities": a bus method the
// plugin may call, optionally confined to filesystem paths.
//
// Two JSON forms are accepted:
//
//	"agents.list"                                   // verb only, no path scope
//	{ "method": "fs.read", "paths": ["${pluginDir}"] }  // path-scoped
//
// A filesystem-scoped method (fs.*, search.project — see capspec) MUST use the
// object form and declare paths; the loader rejects an unscoped one, so a plugin
// can never obtain unrestricted host filesystem access. Supported path tokens:
// "${pluginDir}" (the plugin's own install dir) and absolute paths. Other tokens
// resolve to nothing at registration and therefore grant nothing (fail closed).
type Capability struct {
	Method string   `json:"method"`
	Paths  []string `json:"paths,omitempty"`
}

// UnmarshalJSON accepts either a bare string (verb-only) or the object form.
func (c *Capability) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err == nil {
		c.Method, c.Paths = s, nil
		return nil
	}
	type alias Capability // avoid recursing into this method
	var a alias
	if err := json.Unmarshal(data, &a); err != nil {
		return err
	}
	*c = Capability(a)
	return nil
}

// SettingDef declares one configurable setting a plugin exposes. The host renders
// a control for it in Settings, persists the value, and delivers the current
// values into the plugin's webview (window.__WKS_SETTINGS__ + a wks-settings
// event), so a plugin can be configured without shipping its own settings UI.
type SettingDef struct {
	Key     string   `json:"key"`               // stable id, e.g. "vimMode"
	Label   string   `json:"label"`             // human-readable label
	Type    string   `json:"type"`              // boolean | number | string | select
	Default any      `json:"default,omitempty"` // default value
	Options []string `json:"options,omitempty"` // allowed values when type == "select"
	Help    string   `json:"help,omitempty"`    // optional one-line description
}

// SettingType values.
const (
	SettingBoolean = "boolean"
	SettingNumber  = "number"
	SettingString  = "string"
	SettingSelect  = "select"
)

// ServerSpec describes the plugin's sidecar process.
type ServerSpec struct {
	Command string   `json:"command"`
	Args    []string `json:"args,omitempty"`
	Port    int      `json:"port,omitempty"`   // port the sidecar serves on
	Health  string   `json:"health,omitempty"` // health path, e.g. "/healthz"
}

// PaneContribution is a pane type the plugin injects into the UI. The host
// renders it as a webview at http://127.0.0.1:<port><path>.
type PaneContribution struct {
	Type  string `json:"type"` // unique pane type id, e.g. "acme.tracker"
	Title string `json:"title"`
	Icon  string `json:"icon,omitempty"`
	Path  string `json:"path,omitempty"` // URL path, e.g. "/ui"
	// Scope: where the pane can live.
	//   "global" — cross-agent only (Overview workspace), e.g. a dashboard
	//   "agent"  — inside an agent workspace (gets that agent's sessionId/cwd)
	//   "both"   — either; opens where the user currently is (default)
	Scope string `json:"scope,omitempty"`
}

// HotkeyContribution binds a key to a command. Command is either
// "open-pane:<paneType>" or "emit:<eventType>".
type HotkeyContribution struct {
	ID      string `json:"id"`
	Default string `json:"default"` // e.g. "ctrl+shift+i"
	Command string `json:"command"`
}

// Validate reports the first problem with a manifest, or nil if it's usable.
func (m *Manifest) Validate() error {
	if m.ID == "" {
		return fmt.Errorf("missing id")
	}
	if m.APIVersion != APIVersion {
		return fmt.Errorf("unsupported apiVersion %q (want %q)", m.APIVersion, APIVersion)
	}
	if m.Server != nil && m.Server.Command == "" {
		return fmt.Errorf("server.command is required when server is set")
	}
	seen := map[string]bool{}
	for _, p := range m.Panes {
		if p.Type == "" {
			return fmt.Errorf("pane with empty type")
		}
		if seen[p.Type] {
			return fmt.Errorf("duplicate pane type %q", p.Type)
		}
		seen[p.Type] = true
	}
	// A pane has to be served from somewhere: a sidecar (server) or hub-served
	// static assets (ui). Without either, the webview would have no URL to load.
	if len(m.Panes) > 0 && m.Server == nil && m.UI == "" {
		return fmt.Errorf("plugin has panes but neither a server nor a ui directory to serve them")
	}
	keys := map[string]bool{}
	for _, s := range m.Settings {
		if s.Key == "" {
			return fmt.Errorf("setting with empty key")
		}
		if keys[s.Key] {
			return fmt.Errorf("duplicate setting key %q", s.Key)
		}
		keys[s.Key] = true
		switch s.Type {
		case SettingBoolean, SettingNumber, SettingString:
		case SettingSelect:
			if len(s.Options) == 0 {
				return fmt.Errorf("setting %q is a select but declares no options", s.Key)
			}
		default:
			return fmt.Errorf("setting %q has unknown type %q", s.Key, s.Type)
		}
	}
	for _, c := range m.Capabilities {
		if c.Method == "" {
			return fmt.Errorf("capability with empty method")
		}
		// A filesystem-scoped capability must declare paths — otherwise it would
		// grant unrestricted host filesystem access, which is exactly what the
		// sandbox exists to prevent.
		if _, scoped := capspec.IsPathScoped(c.Method); scoped && len(c.Paths) == 0 {
			return fmt.Errorf("capability %q is filesystem-scoped and must declare \"paths\"", c.Method)
		}
	}
	return nil
}

// Load reads and validates a single plugin.json file.
func Load(path string) (Manifest, error) {
	var m Manifest
	data, err := os.ReadFile(path)
	if err != nil {
		return m, err
	}
	if err := json.Unmarshal(data, &m); err != nil {
		return m, fmt.Errorf("%s: %w", path, err)
	}
	m.Dir = filepath.Dir(path)
	// Loader-owned sidecar state (overrides anything an author put in plugin.json).
	m.Source = readSidecar(filepath.Join(m.Dir, sourceFile))
	m.Disabled = fileExists(filepath.Join(m.Dir, disabledFile))
	if err := m.Validate(); err != nil {
		return m, fmt.Errorf("%s: %w", path, err)
	}
	return m, nil
}

// readSidecar returns the trimmed contents of a sidecar marker file, or "".
func readSidecar(path string) string {
	b, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(b))
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

// LoadDir scans dir for <subdir>/plugin.json files. It returns the valid
// manifests plus a slice of errors for any that failed (so one bad plugin
// doesn't sink the rest). A missing dir yields no manifests and no error.
func LoadDir(dir string) ([]Manifest, []error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, []error{err}
	}
	var manifests []Manifest
	var errs []error
	for _, e := range entries {
		// Dot-prefixed entries are the installer's work dirs (.install-* temps,
		// .trash-* replaced installs awaiting deletion) — never plugins. Without
		// this, a crashed or lock-delayed install leaves a dir whose plugin.json
		// would load as a duplicate of the real plugin on the next boot.
		if strings.HasPrefix(e.Name(), ".") {
			continue
		}
		// Follow a directory symlink as well as a real subdirectory: `workspacer
		// plugin dev` isolates a single plugin by symlinking the developer's dir
		// into a throwaway plugins dir, and os.ReadDir reports a symlink's own type
		// (IsDir() == false) rather than its target's. Only one level is scanned
		// here (no recursion), so following the link can't loop.
		if !e.IsDir() {
			if e.Type()&os.ModeSymlink == 0 {
				continue
			}
			if info, err := os.Stat(filepath.Join(dir, e.Name())); err != nil || !info.IsDir() {
				continue
			}
		}
		path := filepath.Join(dir, e.Name(), "plugin.json")
		if _, statErr := os.Stat(path); statErr != nil {
			continue // not a plugin dir
		}
		m, loadErr := Load(path)
		if loadErr != nil {
			errs = append(errs, loadErr)
			continue
		}
		manifests = append(manifests, m)
	}
	return manifests, errs
}
