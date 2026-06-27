package main

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

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

func TestDefaultConfigParses(t *testing.T) {
	cfg := defaultConfig()
	if len(cfg) == 0 {
		t.Fatal("default config failed to parse (embedded JSON is malformed)")
	}
	kb, _ := cfg["keybindings"].(map[string]any)
	sc, _ := kb["shortcuts"].(map[string]any)
	// The backtick-bearing binding is the one most likely to break the literal.
	if sc["toggle-terminal"] != "ctrl+`" {
		t.Fatalf("toggle-terminal = %v, want ctrl+`", sc["toggle-terminal"])
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
