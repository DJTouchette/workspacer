package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
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

// TestConfigSaveReplacesCustomThemesWholesale proves the Go brain's save()
// matches configService.ts: ui.customThemes is the whole truth when the caller
// sends it, so deleting a theme (sending the full map minus one entry) must
// actually remove it. A plain deep-merge would resurrect the omitted key from
// the cached/on-disk map. Covers the customThemes-resurrection bug (idx 7/23).
func TestConfigSaveReplacesCustomThemesWholesale(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	c := newConfigService()
	// Seed two user-created custom themes.
	c.save(map[string]any{"ui": map[string]any{"customThemes": map[string]any{
		"custom:one": map[string]any{"name": "One"},
		"custom:two": map[string]any{"name": "Two"},
	}}})

	// The theme maker deletes custom:one by sending the full map minus that entry.
	merged := c.save(map[string]any{"ui": map[string]any{"customThemes": map[string]any{
		"custom:two": map[string]any{"name": "Two"},
	}}})

	ct := merged["ui"].(map[string]any)["customThemes"].(map[string]any)
	if _, ok := ct["custom:one"]; ok {
		t.Errorf("deleted theme custom:one should be gone from merged result, got %v", ct)
	}
	if _, ok := ct["custom:two"]; !ok {
		t.Errorf("custom:two should survive, got %v", ct)
	}

	// And it must be gone from disk too (a fresh service reading it back).
	fresh := newConfigService().get()
	fct := fresh["ui"].(map[string]any)["customThemes"].(map[string]any)
	if _, ok := fct["custom:one"]; ok {
		t.Errorf("deleted theme resurrected after reload from disk: %v", fct)
	}
	if _, ok := fct["custom:two"]; !ok {
		t.Errorf("custom:two should persist to disk, got %v", fct)
	}
}

// TestConfigSaveReplacesBudgetsWholesale proves the Go brain's save() matches
// configService.ts: claude.budgets is a user-owned map (Record<sessionId, number>)
// and is the whole truth when the caller sends it, so clearing a per-session
// budget (sending the full map minus one entry, or an empty map) must actually
// remove it. A plain deep-merge would resurrect the omitted key from the
// cached/on-disk map, silently undoing the clear for every web/mobile/remote
// client routed through the hub bus. Covers idx 7/16/24.
func TestConfigSaveReplacesBudgetsWholesale(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	c := newConfigService()
	// Seed two per-session budgets.
	c.save(map[string]any{"claude": map[string]any{"budgets": map[string]any{
		"sessA": float64(5),
		"sessB": float64(10),
	}}})

	// A remote client clears sessB by sending the full budgets map minus that entry.
	merged := c.save(map[string]any{"claude": map[string]any{"budgets": map[string]any{
		"sessA": float64(5),
	}}})

	b := merged["claude"].(map[string]any)["budgets"].(map[string]any)
	if _, ok := b["sessB"]; ok {
		t.Errorf("cleared budget sessB should be gone from merged result, got %v", b)
	}
	if _, ok := b["sessA"]; !ok {
		t.Errorf("sessA should survive, got %v", b)
	}

	// And it must be gone from disk too (a fresh service reading it back).
	fresh := newConfigService().get()
	fb := fresh["claude"].(map[string]any)["budgets"].(map[string]any)
	if _, ok := fb["sessB"]; ok {
		t.Errorf("cleared budget resurrected after reload from disk: %v", fb)
	}
	if _, ok := fb["sessA"]; !ok {
		t.Errorf("sessA should persist to disk, got %v", fb)
	}

	// Clearing ALL budgets (empty map) must also stick.
	merged2 := c.save(map[string]any{"claude": map[string]any{"budgets": map[string]any{}}})
	b2 := merged2["claude"].(map[string]any)["budgets"].(map[string]any)
	if len(b2) != 0 {
		t.Errorf("clearing all budgets should leave an empty map, got %v", b2)
	}
}

// TestLoadFromDiskMigratesStaleNestedChords proves the Go brain's read-time
// migration upgrades stale nested-default chords the way the desktop's
// migrateFlatChords does. A config that postdates the schema rewrite (has a
// prefix, no mode/leader, so migrateKeybindings leaves it alone) but predates
// chord flattening keeps 'prefix t w' for close-pane; the brain must rewrite it
// to the current flat 'prefix w'. Covers idx 8.
func TestLoadFromDiskMigratesStaleNestedChords(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	yamlDoc := "keybindings:\n  prefix: ctrl+a\n  shortcuts:\n    close-pane: prefix t w\n"
	if err := os.WriteFile(p, []byte(yamlDoc), 0o644); err != nil {
		t.Fatal(err)
	}

	c := newConfigService()
	kb, ok := c.get()["keybindings"].(map[string]any)
	if !ok {
		t.Fatal("keybindings missing from loaded config")
	}
	shortcuts, ok := kb["shortcuts"].(map[string]any)
	if !ok {
		t.Fatal("keybindings.shortcuts missing from loaded config")
	}
	if got := shortcuts["close-pane"]; got != "prefix w" {
		t.Fatalf("stale nested chord not migrated: close-pane = %v, want \"prefix w\"", got)
	}
}

// TestConfigDoesNotClobberUnreadableFile proves loadFromDisk must NOT overwrite
// an existing config.yaml with defaults when the file is present but unreadable
// (EACCES). Only ENOENT may seed defaults. Covers idx 21 (data loss).
func TestConfigDoesNotClobberUnreadableFile(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	const userYAML = "ui:\n  theme: mytheme\n"
	if err := os.WriteFile(p, []byte(userYAML), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(p, 0o200); err != nil { // write-only: read error is NOT ENOENT
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(p, 0o644) })
	if _, err := os.ReadFile(p); err == nil {
		t.Skip("running as root: cannot make file unreadable, skipping")
	}

	newConfigService() // loadFromDisk on an existing-but-unreadable config

	_ = os.Chmod(p, 0o644)
	after, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(after), "mytheme") {
		t.Fatalf("unreadable config was clobbered with defaults (data loss); on-disk now:\n%s", string(after))
	}
}

// TestConfigSaveDoesNotClobberUnparseableFile proves a save() issued while
// config.yaml is unparseable does NOT overwrite the user's file with
// defaults+partial. Mirrors the desktop persistBlocked guard. Covers idx 22.
func TestConfigSaveDoesNotClobberUnparseableFile(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	broken := "ui:\n  theme: solarized\nkeybindings: [1, 2\n"
	if err := os.WriteFile(p, []byte(broken), 0o644); err != nil {
		t.Fatal(err)
	}

	c := newConfigService()
	c.save(map[string]any{"editor": map[string]any{"vim": true}})

	after, err := os.ReadFile(p)
	if err != nil {
		t.Fatalf("config.yaml disappeared: %v", err)
	}
	if string(after) != broken {
		t.Fatalf("save() overwrote the unparseable config.yaml, discarding the user's settings.\n got: %q\nwant: %q", string(after), broken)
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
