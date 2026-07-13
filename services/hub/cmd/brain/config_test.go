package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

// TestDeepMergeContractCases runs the shared cross-language deepMerge fixture
// (contracts/deepmerge-cases.json) through the Go deepMerge. The SAME fixture is
// consumed by a configService.ts test, so this is the drift guard keeping the
// two config.yaml deepMerge implementations (TS + Go) in agreement. JSON numbers
// unmarshal to float64 on both the actual and expected sides, so reflect.DeepEqual
// is clean.
func TestDeepMergeContractCases(t *testing.T) {
	// Resolve repo root from this test's package dir (services/hub/cmd/brain).
	path := filepath.Join("..", "..", "..", "..", "contracts", "deepmerge-cases.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read contract fixture %s: %v", path, err)
	}
	var fixture struct {
		Cases []struct {
			Name     string          `json:"name"`
			Target   json.RawMessage `json:"target"`
			Source   json.RawMessage `json:"source"`
			Expected json.RawMessage `json:"expected"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parse contract fixture: %v", err)
	}
	if len(fixture.Cases) == 0 {
		t.Fatal("contract fixture has no cases")
	}
	for _, c := range fixture.Cases {
		t.Run(c.Name, func(t *testing.T) {
			var target, source, expected map[string]any
			if err := json.Unmarshal(c.Target, &target); err != nil {
				t.Fatalf("unmarshal target: %v", err)
			}
			if err := json.Unmarshal(c.Source, &source); err != nil {
				t.Fatalf("unmarshal source: %v", err)
			}
			if err := json.Unmarshal(c.Expected, &expected); err != nil {
				t.Fatalf("unmarshal expected: %v", err)
			}
			got := deepMerge(target, source)
			if !reflect.DeepEqual(got, expected) {
				t.Errorf("deepMerge mismatch\n got: %#v\nwant: %#v", got, expected)
			}
		})
	}
}

func TestConfigGetReloadsOnExternalChange(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	c := newConfigService()
	if ui := c.get()["ui"].(map[string]any); ui["theme"] != "dark" {
		t.Fatalf("initial theme should be the default dark, got %v", ui["theme"])
	}

	// Simulate the desktop app rewriting config.yaml in its own process.
	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.WriteFile(p, []byte("ui:\n  theme: external\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// Force a strictly newer mtime, independent of filesystem timestamp resolution.
	future := time.Now().Add(2 * time.Second)
	_ = os.Chtimes(p, future, future)

	ui := c.get()["ui"].(map[string]any)
	if ui["theme"] != "external" {
		t.Fatalf("get() should reflect the external change, got %v", ui["theme"])
	}
	// Defaults still merge under the externally-written partial.
	if c.get()["terminal"] == nil {
		t.Error("defaults should still merge over the external file")
	}
}

// TestConfigSaveFoldsInExternalChange proves save() re-reads a config.yaml that
// was changed under it (e.g. by the desktop app) before merging its own partial,
// instead of clobbering that change with a stale in-memory cache. get() is
// mtime-gated for exactly this reason; save() must honour the same gate.
func TestConfigSaveFoldsInExternalChange(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	c := newConfigService()
	// Prime the in-memory cache — theme defaults to dark.
	if ui := c.get()["ui"].(map[string]any); ui["theme"] != "dark" {
		t.Fatalf("precondition: default theme should be dark, got %v", ui["theme"])
	}

	// The desktop app rewrites config.yaml in its own process, changing the theme.
	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.WriteFile(p, []byte("ui:\n  theme: external\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	future := time.Now().Add(2 * time.Second) // strictly newer, independent of fs resolution
	_ = os.Chtimes(p, future, future)

	// The brain saves an unrelated partial. It must fold the external theme in,
	// not overwrite it from the stale cache.
	merged := c.save(map[string]any{"editor": map[string]any{"vim": true}})

	if ui := merged["ui"].(map[string]any); ui["theme"] != "external" {
		t.Errorf("save clobbered the external theme: got %v, want external", ui["theme"])
	}
	if ed := merged["editor"].(map[string]any); ed["vim"] != true {
		t.Errorf("save dropped its own partial: editor.vim = %v", ed["vim"])
	}
	// Confirm it's what actually landed on disk.
	fresh := newConfigService().get()
	if ui := fresh["ui"].(map[string]any); ui["theme"] != "external" {
		t.Errorf("persisted theme = %v, want external (external write was clobbered)", ui["theme"])
	}
}

func TestDefaultConfigParses(t *testing.T) {
	cfg := defaultConfig()
	if len(cfg) == 0 {
		t.Fatal("default config failed to parse (embedded JSON is malformed)")
	}
	kb, _ := cfg["keybindings"].(map[string]any)
	sc, _ := kb["shortcuts"].(map[string]any)
	// The backtick-bearing binding is the one most likely to break the literal.
	if sc["toggle-terminal"] != "mod+`" {
		t.Fatalf("toggle-terminal = %v, want mod+`", sc["toggle-terminal"])
	}
	if ui, _ := cfg["ui"].(map[string]any); ui["theme"] != "dark" {
		t.Fatalf("ui.theme = %v, want dark", ui["theme"])
	}
}

func TestDeepMergePreservesDefaultsAndSkipsNull(t *testing.T) {
	target := map[string]any{
		"ui": map[string]any{"theme": "dark", "fontSize": float64(14)},
		"x":  float64(1),
	}
	source := map[string]any{
		"ui": map[string]any{"theme": "light", "fontSize": nil}, // null = keep default
		"y":  float64(2),
	}
	got := deepMerge(target, source)
	ui := got["ui"].(map[string]any)
	if ui["theme"] != "light" {
		t.Errorf("theme should be overridden to light, got %v", ui["theme"])
	}
	if ui["fontSize"] != float64(14) {
		t.Errorf("null source must keep the default fontSize, got %v", ui["fontSize"])
	}
	if got["x"] != float64(1) || got["y"] != float64(2) {
		t.Errorf("siblings should survive, got x=%v y=%v", got["x"], got["y"])
	}
	// Target must not be mutated.
	if target["ui"].(map[string]any)["theme"] != "dark" {
		t.Error("deepMerge mutated its target")
	}
}

func TestConfigSaveReloadRoundTrip(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	c := newConfigService()
	c.save(map[string]any{"ui": map[string]any{"theme": "nord"}})

	// Persisted as YAML at the canonical path.
	if _, err := os.Stat(filepath.Join(dir, "workspacer", "config.yaml")); err != nil {
		t.Fatalf("config.yaml not written: %v", err)
	}

	// A fresh service reads it back, with untouched defaults intact.
	fresh := newConfigService()
	cfg := fresh.get()
	ui := cfg["ui"].(map[string]any)
	if ui["theme"] != "nord" {
		t.Errorf("theme should persist as nord, got %v", ui["theme"])
	}
	if ui["fontSize"] == nil {
		t.Error("default fontSize should survive a partial save")
	}
}

func TestMigrateKeybindingsLegacyVim(t *testing.T) {
	cfg := defaultConfig()
	cfg["keybindings"] = map[string]any{"mode": "vim", "leader": "space"} // legacy shape
	migrated := migrateKeybindings(cfg)

	kb := migrated["keybindings"].(map[string]any)
	if kb["prefix"] != "ctrl+space" {
		t.Errorf("legacy keybindings should reset to prefix scheme, got %v", kb["prefix"])
	}
	if _, hasMode := kb["mode"]; hasMode {
		t.Error("migrated keybindings should drop legacy mode")
	}
	ed := migrated["editor"].(map[string]any)
	if ed["vim"] != true {
		t.Error("vim mode should be preserved as editor.vim")
	}
}

func TestListModelsReadsConfigDefault(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	// Seed a config with a default model + a persisted seen model.
	newConfigService().save(map[string]any{
		"claude": map[string]any{"defaultModel": "opus", "seenModels": []any{"sonnet"}},
	})

	// claudemon unreachable → liveModels empty, seen comes from config only.
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:0"))
	res := reg.listModels(context.Background())
	if res.DefaultModel != "opus" {
		t.Errorf("defaultModel = %q, want opus", res.DefaultModel)
	}
	if len(res.Seen) != 1 || res.Seen[0] != "sonnet" {
		t.Errorf("seen = %v, want [sonnet]", res.Seen)
	}
	if len(res.Aliases) != 4 {
		t.Errorf("expected 4 aliases, got %d", len(res.Aliases))
	}
}

// TestEmbeddedDefaultsAreCompleteAndParse guards the go:embed of
// config_defaults.json (the single source of truth shared with the desktop):
// it must parse and carry every top-level section — including agents/updates and
// the claude fields the old hand-transcribed copy was missing, which is what let
// web/mobile fall back to different values than the desktop.
func TestEmbeddedDefaultsAreCompleteAndParse(t *testing.T) {
	def := defaultConfig()
	if len(def) == 0 {
		t.Fatal("embedded defaultConfigJSON parsed to an empty map — go:embed not wired?")
	}
	for _, section := range []string{
		"ui", "terminal", "browser", "panes", "keybindings", "notifications",
		"editor", "claude", "agents", "supervisor", "directories", "scripts",
		"session", "updates", "apps",
	} {
		if _, ok := def[section]; !ok {
			t.Errorf("default config missing top-level section %q", section)
		}
	}
	claude, _ := def["claude"].(map[string]any)
	if claude["transport"] != "stream" {
		t.Errorf("claude.transport = %v, want stream", claude["transport"])
	}
	if _, ok := def["agents"].(map[string]any)["binaries"]; !ok {
		t.Error("agents.binaries missing from defaults")
	}
}
