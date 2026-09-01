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

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// TestDeepMergeContractCases runs the shared cross-language deepMerge fixture
// (contracts/deepmerge-cases.json) through the Go deepMerge. The SAME fixture is
// consumed by a configService.ts test, so this is the drift guard keeping the
// two config.yaml deepMerge implementations (TS + Go) in agreement. JSON numbers
// unmarshal to float64 on both the actual and expected sides, so reflect.DeepEqual
// is clean.
func TestDeepMergeContractCases(t *testing.T) {
	// mustReadRepoFile, not os.ReadFile of a "../../../.." path: contracts/
	// lives above this module and cmd/go's test cache drops out-of-module
	// inputs from the key, so a direct read leaves this guard printing
	// `ok (cached)` over a fixture it never looked at. See repofile_test.go.
	const path = "contracts/deepmerge-cases.json"
	data := mustReadRepoFile(t, "contracts", "deepmerge-cases.json")
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
	// deepMergeFloor is the corpus's size today: "not zero" is met by a fixture
	// that lost every case but one, and this corpus is the only thing holding
	// the Go merge to the TypeScript one.
	const deepMergeFloor = 10
	var tally sweepguard.Tally
	for _, c := range fixture.Cases {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
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
	if err := tally.RequireEvery("the deepMerge corpus", deepMergeFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

func TestConfigGetReloadsOnExternalChange(t *testing.T) {
	dir := tempConfigHome(t)

	c := newConfigService()
	if ui := c.get()["ui"].(map[string]any); ui["theme"] != "everforest" {
		t.Fatalf("initial theme should be the default everforest, got %v", ui["theme"])
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
	dir := tempConfigHome(t)

	c := newConfigService()
	// Prime the in-memory cache — theme defaults to everforest.
	if ui := c.get()["ui"].(map[string]any); ui["theme"] != "everforest" {
		t.Fatalf("precondition: default theme should be everforest, got %v", ui["theme"])
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
	merged := mustSave(t, c, map[string]any{"editor": map[string]any{"vim": true}})

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
	if ui, _ := cfg["ui"].(map[string]any); ui["theme"] != "everforest" {
		t.Fatalf("ui.theme = %v, want everforest", ui["theme"])
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
	dir := tempConfigHome(t)

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

func TestManagerSelectionMigrationAndNullableRoundTrip(t *testing.T) {
	dir := tempConfigHome(t)
	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte("customTop: keep-me\nagents:\n  managerModels:\n    claude: opus[1m]\n    codex: gpt-5-codex\n  managerContextWindows:\n    codex: 400000\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	c := newConfigService()
	agents := c.get()["agents"].(map[string]any)
	models := agents["managerModels"].(map[string]any)
	contexts := agents["managerContextWindows"].(map[string]any)
	if models["claude"] != "opus" || contexts["claude"] != uint64(1_000_000) {
		t.Fatalf("legacy manager selection was not canonicalized: models=%#v contexts=%#v", models, contexts)
	}
	if c.get()["customTop"] != "keep-me" {
		t.Fatal("migration dropped unrelated config")
	}

	merged := mustSave(t, c, map[string]any{"agents": map[string]any{
		"managerContextWindows": map[string]any{"codex": nil},
	}})
	contexts = merged["agents"].(map[string]any)["managerContextWindows"].(map[string]any)
	if value, present := contexts["codex"]; !present || value != nil {
		t.Fatalf("explicit Codex provider-default did not survive: %#v", contexts)
	}
	if contexts["claude"] != uint64(1_000_000) {
		t.Fatalf("changing Codex erased Claude: %#v", contexts)
	}
	fresh := newConfigService().get()["agents"].(map[string]any)["managerContextWindows"].(map[string]any)
	if value, present := fresh["codex"]; !present || value != nil {
		t.Fatalf("Codex null did not round-trip through YAML: %#v", fresh)
	}
}

func TestManagerSelectionSaveRejectsProviderManagedContext(t *testing.T) {
	tempConfigHome(t)
	c := newConfigService()
	_, err := c.save(map[string]any{"agents": map[string]any{
		"managerContextWindows": map[string]any{"copilot": 1_000_000},
	}})
	if err == nil || !strings.Contains(err.Error(), "unsupported-context-window") {
		t.Fatalf("got %v, want unsupported-context-window refusal", err)
	}
}

func TestInvalidDefaultModelTypeResetsOnlySelectionAndKeepsPersistenceLive(t *testing.T) {
	for _, tc := range []struct {
		name  string
		value string
	}{
		{"null", "null"},
		{"non-string", "42"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			dir := tempConfigHome(t)
			p := filepath.Join(dir, "workspacer", "config.yaml")
			if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
				t.Fatal(err)
			}
			raw := "customTop: keep-me\nui:\n  theme: light\nclaude:\n  defaultModel: " + tc.value + "\n"
			if err := os.WriteFile(p, []byte(raw), 0o644); err != nil {
				t.Fatal(err)
			}

			c := newConfigService()
			cfg := c.get()
			if cfg["customTop"] != "keep-me" || cfg["ui"].(map[string]any)["theme"] != "light" {
				t.Fatalf("valid YAML outside the model pair was not preserved: %#v", cfg)
			}
			claude := cfg["claude"].(map[string]any)
			if claude["defaultModel"] != "opus" || claude["contextWindow"] != float64(1_000_000) {
				t.Fatalf("invalid pair did not reset to the shipped canonical pair: %#v", claude)
			}
			if c.persistBlocked {
				t.Fatal("valid YAML with an invalid model pair must not block later saves")
			}
			if backups, err := filepath.Glob(p + ".broken-*"); err != nil || len(backups) != 0 {
				t.Fatalf("model validation minted a broken-YAML backup: %v (%v)", backups, err)
			}

			mustSave(t, c, map[string]any{"ui": map[string]any{"fontSize": 16}})
			fresh := newConfigService().get()
			if fresh["customTop"] != "keep-me" || fresh["ui"].(map[string]any)["fontSize"] != 16 {
				t.Fatalf("subsequent save was blocked or discarded valid YAML: %#v", fresh)
			}
		})
	}
}

// TestConfigSaveIgnoresUpdatesFromTheBus: `updates` is host-trusted. Every
// config.save the brain answers came off the bus (a web/remote client, a plugin,
// an agent via the MCP facade), and updates.channel is string-concatenated into
// the electron-updater feed URL the desktop then downloads and installs from —
// so honouring it hands a remote caller persistent code execution wearing the
// app's own update dialog. The rest of the same save must still apply, or a
// client that happens to echo the whole config back loses every setting.
func TestConfigSaveIgnoresUpdatesFromTheBus(t *testing.T) {
	tempConfigHome(t)

	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	params := `{"ui":{"theme":"nord"},"updates":{"channel":"../../attacker/repo","enabled":false}}`
	if _, err := reg.handle(context.Background(), "config.save", json.RawMessage(params)); err != nil {
		t.Fatal(err)
	}

	// Read from disk, not from the in-memory merge: the file is what the desktop
	// updater reads at boot.
	onDisk := newConfigService().get()
	updates, _ := onDisk["updates"].(map[string]any)
	if updates == nil {
		t.Fatal("updates section missing from the persisted config")
	}
	if updates["channel"] != "latest" {
		t.Errorf("a bus config.save rewrote updates.channel to %v — the updater feed is caller-controlled", updates["channel"])
	}
	if updates["enabled"] != true {
		t.Errorf("a bus config.save disabled auto-update (enabled=%v)", updates["enabled"])
	}
	if ui, _ := onDisk["ui"].(map[string]any); ui["theme"] != "nord" {
		t.Errorf("the rest of the save must still persist, theme = %v", ui["theme"])
	}
}

func TestMigrateKeybindingsLegacyVim(t *testing.T) {
	// migrateKeybindings does not just transform the map — it PERSISTS it, via a
	// direct writeConfigYAML outside the lock and the CAS. Without this redirect
	// the line below builds a bare-defaults config and this test publishes it
	// over the developer's real ~/.config/workspacer/config.yaml, wiping every
	// setting on the machine on every `go test ./...`. That is not a
	// hypothetical: it is what it did until 2026-08-23.
	tempConfigHome(t)

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
	// editor.vim is no longer written: the field died with the in-app
	// CodeMirror editor — nothing reads it, so the migration stopped
	// preserving legacy vim mode there.
	if ed, ok := migrated["editor"].(map[string]any); ok {
		if _, hasVim := ed["vim"]; hasVim {
			t.Error("migration must not write the dead editor.vim field")
		}
	}
}

// TestConfigSaveReplacesCustomThemesWholesale proves the Go brain's save()
// matches configService.ts: ui.customThemes is the whole truth when the caller
// sends it, so deleting a theme (sending the full map minus one entry) must
// actually remove it. A plain deep-merge would resurrect the omitted key from
// the cached/on-disk map. Covers the customThemes-resurrection bug (idx 7/23).
func TestConfigSaveReplacesCustomThemesWholesale(t *testing.T) {
	tempConfigHome(t)

	c := newConfigService()
	// Seed two user-created custom themes.
	c.save(map[string]any{"ui": map[string]any{"customThemes": map[string]any{
		"custom:one": map[string]any{"name": "One"},
		"custom:two": map[string]any{"name": "Two"},
	}}})

	// The theme maker deletes custom:one by sending the full map minus that entry.
	merged := mustSave(t, c, map[string]any{"ui": map[string]any{"customThemes": map[string]any{
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
	tempConfigHome(t)

	c := newConfigService()
	// Seed two per-session budgets.
	c.save(map[string]any{"claude": map[string]any{"budgets": map[string]any{
		"sessA": float64(5),
		"sessB": float64(10),
	}}})

	// A remote client clears sessB by sending the full budgets map minus that entry.
	merged := mustSave(t, c, map[string]any{"claude": map[string]any{"budgets": map[string]any{
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
	merged2 := mustSave(t, c, map[string]any{"claude": map[string]any{"budgets": map[string]any{}}})
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
	dir := tempConfigHome(t)

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
	dir := tempConfigHome(t)

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
	dir := tempConfigHome(t)

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

// TestConfigSaveDoesNotClobberEmptyOrCommentOnlyFile proves the Go twin of the
// desktop configService.ts fail-safe: yaml.Unmarshal of an empty or
// comment-only config.yaml succeeds with a NIL map (not an error), so without
// an explicit guard deepMerge(defaults, nil) would silently hand back
// untouched defaults and the next save would write those bare defaults over
// the user's real config. Assert the refusal, not just the parse result —
// that's the property that protects the user.
func TestConfigSaveDoesNotClobberEmptyOrCommentOnlyFile(t *testing.T) {
	for _, tc := range []struct {
		name     string
		contents string
	}{
		{"empty file", ""},
		{"comment-only file", "# just a comment\n"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			p := seedConfig(t, tc.contents)

			c := newConfigService()
			c.save(map[string]any{"editor": map[string]any{"vim": true}})

			after, err := os.ReadFile(p)
			if err != nil {
				t.Fatalf("config.yaml disappeared: %v", err)
			}
			if string(after) != tc.contents {
				t.Fatalf("save() overwrote a %s with defaults, discarding whatever the user actually had.\n got: %q\nwant: %q", tc.name, string(after), tc.contents)
			}
		})
	}
}

// TestConfigENOENTAfterLoadIsNotFirstRun proves a config.yaml that disappears
// mid-run (e.g. a hand edit that truncates before rewriting) is NOT treated as
// "no config yet": the singleton already has one, so ENOENT here must keep the
// existing in-memory config and block saves, not reseed bare defaults that the
// next save would then persist over whatever the user actually has.
func TestConfigENOENTAfterLoadIsNotFirstRun(t *testing.T) {
	p := seedConfig(t, "ui:\n  theme: solarized\n")

	c := newConfigService()
	before := c.current

	if err := os.Remove(p); err != nil {
		t.Fatal(err)
	}
	got := c.reload()

	if !reflect.DeepEqual(got, before) {
		t.Fatalf("ENOENT after a config was already loaded reseeded defaults instead of keeping the existing config.\n got: %#v\nwant: %#v", got, before)
	}
	if !c.persistBlocked {
		t.Fatal("ENOENT after a config was already loaded did not block saves")
	}

	c.save(map[string]any{"editor": map[string]any{"vim": true}})
	if _, err := os.ReadFile(p); err == nil {
		t.Fatal("save() reseeded config.yaml after a mid-run disappearance instead of refusing to write")
	}
}

func TestListModelsReadsConfigDefault(t *testing.T) {
	tempConfigHome(t)
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
	// The alias SET is the contract shared with claudeModels.ts and is pinned by
	// contracts/claude-model-catalog-cases.json (models_contract_test.go); this
	// test only asserts that the wiring reaches it. It used to assert 4, which
	// was the drifted number: the desktop has always returned 6, including the
	// 1M-context opus[1m] / sonnet[1m] rows.
	if len(res.Aliases) != len(buildListModels("", nil, false, "", nil, nil).Aliases) {
		t.Errorf("listModels returned %d aliases, want the contract's %d",
			len(res.Aliases), len(buildListModels("", nil, false, "", nil, nil).Aliases))
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
		"editor", "claude", "agents", "directories", "scripts",
		"updates", "apps",
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

// TestHostTrustedContractCases runs the shared cross-language fixture
// (contracts/host-trusted-config-cases.json) through the Go dropHostTrusted.
// The SAME fixture is consumed by a hostTrustedConfig.ts test in the desktop
// main process. Both answer config.save over the bus — this one under the
// default DELEGATE_CATALOG_TO_BRAIN, the TS one when delegation is off — so a
// section either drops in both or the guard has a hole in whichever half runs.
func TestHostTrustedContractCases(t *testing.T) {
	const path = "contracts/host-trusted-config-cases.json"
	data := mustReadRepoFile(t, "contracts", "host-trusted-config-cases.json")
	var fixture struct {
		Sections []string `json:"sections"`
		Paths    []string `json:"paths"`
		Cases    []struct {
			Name     string         `json:"name"`
			Partial  map[string]any `json:"partial"`
			Expected map[string]any `json:"expected"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parse contract fixture: %v", err)
	}
	if len(fixture.Cases) == 0 {
		t.Fatal("fixture has no cases — a silently empty fixture guards nothing")
	}

	// The section list itself is part of the contract: adding one on only the
	// Go side leaves the desktop's bus handler accepting it.
	if !reflect.DeepEqual(fixture.Sections, hostTrustedSections) {
		t.Errorf("hostTrustedSections = %v, fixture says %v", hostTrustedSections, fixture.Sections)
	}
	// The sub-key list too, or agents.binaries (argv[0] for every spawn) can be
	// dropped from the guard with every case still green — the cases only see
	// the drop, not the list that drives it.
	if !reflect.DeepEqual(fixture.Paths, hostTrustedPaths) {
		t.Errorf("hostTrustedPaths = %v, fixture says %v", hostTrustedPaths, fixture.Paths)
	}

	// hostTrustedFloor: the corpus's size today, for the same reason.
	const hostTrustedFloor = 13
	var tally sweepguard.Tally
	for _, c := range fixture.Cases {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
			got := dropHostTrusted(c.Partial)
			// An empty fixture object unmarshals to an empty (non-nil) map; a
			// dropped-everything result may be either. Compare by length first.
			if len(got) != len(c.Expected) {
				t.Fatalf("dropHostTrusted(%v) = %v, want %v", c.Partial, got, c.Expected)
			}
			for k, want := range c.Expected {
				if !reflect.DeepEqual(got[k], want) {
					t.Errorf("key %q = %v, want %v", k, got[k], want)
				}
			}
			for k := range got {
				if _, ok := c.Expected[k]; !ok {
					t.Errorf("unexpected key %q survived the drop", k)
				}
			}
		})
	}
	if err := tally.RequireEvery("the host-trusted-config corpus", hostTrustedFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}
