package plugin

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// settingsValuesFile holds a plugin's user-set setting VALUES — an overlay over
// the manifest's declared defaults, keyed by setting key. It lives in the
// plugin's own directory alongside the other loader-owned sidecar files
// (.bus-token, .disabled, .install-source), so a plugin's configuration travels
// with it and survives hub restarts. Only the values the user actually changed
// are stored; anything unset falls back to the manifest default on read, keeping
// this a small overlay rather than a full mirror of the schema.
const settingsValuesFile = ".settings.json"

func settingsFilePath(dir string) string { return filepath.Join(dir, settingsValuesFile) }

// GetSettings returns a plugin's current setting values: the manifest-declared
// defaults with the user's persisted overlay applied on top. Every declared
// setting that has a default appears; persisted values override. Keys on disk
// that the manifest no longer declares are dropped — the manifest is the schema
// of record. Errors if the plugin isn't loaded.
func (m *Manager) GetSettings(pluginID string) (map[string]any, error) {
	m.mu.Lock()
	l, ok := m.plugins[pluginID]
	m.mu.Unlock()
	if !ok {
		return nil, fmt.Errorf("plugin %q is not loaded", pluginID)
	}
	m.settingsMu.Lock()
	defer m.settingsMu.Unlock()
	return mergedSettings(l.manifest), nil
}

// SetSettings validates a partial map of setting values against the plugin's
// manifest, merges it into the persisted overlay, writes it, and publishes
// plugin.settings.changed with the full merged values. A nil value for a key
// deletes it (reverting to the manifest default). Validation is all-or-nothing:
// an unknown key or a type/enum mismatch rejects the whole write and nothing is
// persisted. Returns the full merged values (defaults + overlay).
func (m *Manager) SetSettings(pluginID string, partial map[string]any) (map[string]any, error) {
	m.mu.Lock()
	l, ok := m.plugins[pluginID]
	m.mu.Unlock()
	if !ok {
		return nil, fmt.Errorf("plugin %q is not loaded", pluginID)
	}
	mf := l.manifest
	if mf.Dir == "" {
		return nil, fmt.Errorf("plugin %q has no directory on disk to persist settings", pluginID)
	}

	defs := make(map[string]SettingDef, len(mf.Settings))
	for _, s := range mf.Settings {
		defs[s.Key] = s
	}
	// Validate the entire partial before touching disk. Unknown keys are rejected
	// rather than passed through: the manifest declares the plugin's whole settings
	// surface, so a key it doesn't know is a caller bug, and silently persisting it
	// would let stale/typo'd keys accumulate on disk with no schema to prune them.
	for k, v := range partial {
		def, declared := defs[k]
		if !declared {
			return nil, fmt.Errorf("unknown setting key %q for plugin %q", k, pluginID)
		}
		if v == nil {
			continue // delete → revert to default; no type to check
		}
		if err := validateSettingValue(def, v); err != nil {
			return nil, err
		}
	}

	m.settingsMu.Lock()
	overlay := readSettingsOverlay(mf.Dir)
	for k, v := range partial {
		if v == nil {
			delete(overlay, k)
		} else {
			overlay[k] = v
		}
	}
	if err := writeSettingsOverlay(mf.Dir, overlay); err != nil {
		m.settingsMu.Unlock()
		return nil, err
	}
	merged := mergedSettings(mf)
	m.settingsMu.Unlock()

	m.pub.Publish(event.New("plugin.settings.changed", "hub", map[string]any{
		"id":     pluginID,
		"values": merged,
	}))
	// A webview picks the change up live (window.__WKS_SETTINGS__ / the bus
	// event), but a sidecar reads WKS_SETTINGS from its environment at spawn —
	// restart it so the new values actually take effect. Add stops the previous
	// supervisor and starts a fresh one with the updated env.
	if mf.Server != nil && !mf.Disabled {
		m.Add(mf)
	}
	return merged, nil
}

// settingsEnvJSON marshals a plugin's merged setting values for the sidecar's
// WKS_SETTINGS environment variable. Empty string when there is nothing to
// deliver or marshalling fails (the sidecar then falls back to its defaults).
func (m *Manager) settingsEnvJSON(mf Manifest) string {
	m.settingsMu.Lock()
	merged := mergedSettings(mf)
	m.settingsMu.Unlock()
	if len(merged) == 0 {
		return ""
	}
	b, err := json.Marshal(merged)
	if err != nil {
		return ""
	}
	return string(b)
}

// mergedSettings layers a plugin's persisted overlay over its manifest defaults,
// keeping only keys the manifest still declares. Caller holds m.settingsMu.
func mergedSettings(mf Manifest) map[string]any {
	out := make(map[string]any)
	defs := make(map[string]SettingDef, len(mf.Settings))
	for _, s := range mf.Settings {
		defs[s.Key] = s
		if s.Default != nil {
			out[s.Key] = s.Default
		}
	}
	if mf.Dir == "" {
		return out
	}
	for k, v := range readSettingsOverlay(mf.Dir) {
		if _, declared := defs[k]; declared {
			out[k] = v
		}
	}
	return out
}

// validateSettingValue checks a value against its SettingDef's type (and, for a
// select, its option set). Numbers accept any JSON/Go numeric kind so a caller
// passing an int and a value round-tripped through JSON (float64) both pass.
func validateSettingValue(def SettingDef, v any) error {
	switch def.Type {
	case SettingBoolean:
		if _, ok := v.(bool); !ok {
			return fmt.Errorf("setting %q expects a boolean, got %T", def.Key, v)
		}
	case SettingNumber:
		if !isNumber(v) {
			return fmt.Errorf("setting %q expects a number, got %T", def.Key, v)
		}
	case SettingString:
		if _, ok := v.(string); !ok {
			return fmt.Errorf("setting %q expects a string, got %T", def.Key, v)
		}
	case SettingSelect:
		s, ok := v.(string)
		if !ok {
			return fmt.Errorf("setting %q expects one of its options (a string), got %T", def.Key, v)
		}
		for _, opt := range def.Options {
			if opt == s {
				return nil
			}
		}
		return fmt.Errorf("setting %q value %q is not one of the declared options %v", def.Key, s, def.Options)
	default:
		// Should never happen: Manifest.Validate rejects unknown setting types at
		// load, so a loaded plugin's defs are all one of the cases above.
		return fmt.Errorf("setting %q has unknown type %q", def.Key, def.Type)
	}
	return nil
}

func isNumber(v any) bool {
	switch v.(type) {
	case float64, float32,
		int, int8, int16, int32, int64,
		uint, uint8, uint16, uint32, uint64,
		json.Number:
		return true
	}
	return false
}

// readSettingsOverlay reads the plugin's persisted value overlay. A missing or
// unreadable file yields an empty (non-nil) map so callers can range/insert
// without a nil check.
func readSettingsOverlay(dir string) map[string]any {
	out := map[string]any{}
	b, err := os.ReadFile(settingsFilePath(dir))
	if err != nil {
		return out
	}
	if err := json.Unmarshal(b, &out); err != nil || out == nil {
		return map[string]any{}
	}
	return out
}

// writeSettingsOverlay persists the value overlay. 0600 like .bus-token — the
// values may include tokens/keys a plugin needs and aren't meant to be
// world-readable.
func writeSettingsOverlay(dir string, overlay map[string]any) error {
	b, err := json.MarshalIndent(overlay, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(settingsFilePath(dir), b, 0o600)
}
