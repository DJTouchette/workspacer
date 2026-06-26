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
)

// Manifest is a plugin's plugin.json.
type Manifest struct {
	ID         string `json:"id"`
	Name       string `json:"name"`
	APIVersion string `json:"apiVersion"`

	Server  *ServerSpec          `json:"server,omitempty"`
	Panes   []PaneContribution   `json:"panes,omitempty"`
	Hotkeys []HotkeyContribution `json:"hotkeys,omitempty"`

	// Provides: capabilities this plugin answers on the bus.
	// Capabilities: capabilities it may call.
	Provides     []string `json:"provides,omitempty"`
	Capabilities []string `json:"capabilities,omitempty"`
	Emits        []string `json:"emits,omitempty"`
	Consumes     []string `json:"consumes,omitempty"`

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
		if !e.IsDir() {
			continue
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
