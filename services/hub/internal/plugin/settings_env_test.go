package plugin

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// A sidecar receives the plugin's merged setting values as WKS_SETTINGS at
// spawn, and a SetSettings write restarts it so the new values take effect.
func TestSidecarGetsSettingsEnvAndRestartsOnChange(t *testing.T) {
	dir := t.TempDir()
	mf := Manifest{
		ID: "acme.envy", APIVersion: "1", Dir: dir,
		// Dump WKS_SETTINGS to a file in the plugin dir, then idle like a real sidecar.
		Server: &ServerSpec{Command: "sh", Args: []string{"-c", `printf '%s' "$WKS_SETTINGS" > wks-settings.out; sleep 30`}},
		Settings: []SettingDef{
			{Key: "greeting", Label: "Greeting", Type: SettingString, Default: "hello"},
			{Key: "count", Label: "Count", Type: SettingNumber, Default: 2},
		},
	}

	cap := newCapture()
	m := NewManager(cap, nil)
	m.Add(mf)
	defer m.Stop()
	cap.waitFor(t, "sidecar.running")

	out := filepath.Join(dir, "wks-settings.out")
	readEnv := func() map[string]any {
		t.Helper()
		deadline := time.After(3 * time.Second)
		for {
			b, err := os.ReadFile(out)
			if err == nil && len(b) > 0 {
				var v map[string]any
				if jsonErr := json.Unmarshal(b, &v); jsonErr != nil {
					t.Fatalf("WKS_SETTINGS is not JSON: %q", b)
				}
				return v
			}
			select {
			case <-deadline:
				t.Fatalf("sidecar never wrote WKS_SETTINGS (err=%v)", err)
			case <-time.After(20 * time.Millisecond):
			}
		}
	}

	got := readEnv()
	if got["greeting"] != "hello" || got["count"] != float64(2) {
		t.Fatalf("spawn env = %v, want manifest defaults", got)
	}

	// Change a value: the sidecar must be restarted with the merged overlay.
	if err := os.Remove(out); err != nil {
		t.Fatal(err)
	}
	if _, err := m.SetSettings("acme.envy", map[string]any{"greeting": "hi"}); err != nil {
		t.Fatal(err)
	}
	got = readEnv()
	if got["greeting"] != "hi" || got["count"] != float64(2) {
		t.Fatalf("post-change env = %v, want overlay merged over defaults", got)
	}
}
