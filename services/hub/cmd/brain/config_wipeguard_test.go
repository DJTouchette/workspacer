package main

// The WHOLE-CONFIG SERIALIZE plane: what happens when something hands
// writeConfigYAML a complete config document rather than a merge derived from
// disk.
//
// loadFromDisk's fail-safes (persistBlocked on unreadable / unparseable / empty
// / vanished) all guard the same shape: "we ended up holding defaults because
// the READ went wrong". None of them can see a caller that builds a
// defaults-shaped map itself and writes it — which is a real caller, because
// migrateKeybindings/migrateFlatChords/pruneRemovedShortcuts persist through a
// direct writeConfigYAML, outside the cross-process lock, outside the CAS, and
// without consulting persistBlocked.
//
// That is how the developer's live config.yaml was reset to factory defaults
// repeatedly on 2026-08-23: TestMigrateKeybindingsLegacyVim built
// defaultConfig(), stuck a legacy keybindings stub on it, and called
// migrateKeybindings — with no config-home redirect, so every `go test ./...` in
// services/hub published the shipped defaults over the machine's real settings.
// The Go marshaller's fingerprint (sorted keys, 4-space indent) on the clobbered
// file is what identified this writer rather than the desktop's js-yaml twin.

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	yaml "gopkg.in/yaml.v3"
)

// realisticConfig is a populated config.yaml in the shape the desktop writes:
// js-yaml's 2-space insertion order, with several non-default values.
const realisticConfig = `ui:
  theme: everforest
  sidebarWidth: 316
claude:
  skipPermissionsDefault: true
agents:
  fleetFullAccess: true
projects:
  /home/me/work:
    name: work
onboardingDismissed: true
`

// TestWriteConfigYAMLRefusesToWipeAPopulatedConfigWithDefaults is the regression
// for the live incident. It drives the exact caller that did the damage — a
// defaults-shaped map handed straight to a read-time migration — and asserts the
// user's file survives.
//
// Without refuseWipeWithDefaults this fails: config.yaml comes back as the
// shipped defaults, so skipPermissionsDefault flips to false (which is what put
// every dispatched worker back on approval prompts), fleetFullAccess to false,
// projects to {}, and onboardingDismissed disappears entirely.
func TestWriteConfigYAMLRefusesToWipeAPopulatedConfigWithDefaults(t *testing.T) {
	p := seedConfig(t, realisticConfig)

	// The caller that did it, verbatim in shape: a config built from defaults
	// rather than read from disk, persisted by a read-time migration.
	cfg := defaultConfig()
	cfg["keybindings"] = map[string]any{"mode": "vim", "leader": "space"}
	migrateKeybindings(cfg)

	after, err := os.ReadFile(p)
	if err != nil {
		t.Fatalf("config.yaml disappeared: %v", err)
	}
	if string(after) != realisticConfig {
		t.Fatalf("a whole-config serialize of bare defaults replaced a populated config.yaml — total silent loss of every setting the user had.\n got:\n%s\nwant (untouched):\n%s", string(after), realisticConfig)
	}
}

// TestWriteConfigYAMLReportsTheRefusal covers the other half of "loud refusal":
// the error must reach the caller, not be swallowed. saveLocked keys off it to
// keep serving the previous value instead of adopting one that is not on disk.
func TestWriteConfigYAMLReportsTheRefusal(t *testing.T) {
	seedConfig(t, realisticConfig)

	err := writeConfigYAML(defaultConfig())
	if !errors.Is(err, errWipeWithDefaults) {
		t.Fatalf("writeConfigYAML returned %v, want errWipeWithDefaults — a refusal nobody is told about reports a successful save for a file that never changed", err)
	}
}

// TestWriteConfigYAMLStillSeedsDefaultsOnFirstRun is the regression guard on the
// guard. The refusal must be scoped to "there is something there to lose": a
// genuine first run has no config.yaml at all, and must still be seeded and then
// saved normally, or the fix trades data loss for an app that can never persist
// a setting on a fresh machine.
func TestWriteConfigYAMLStillSeedsDefaultsOnFirstRun(t *testing.T) {
	dir := tempConfigHome(t)
	p := filepath.Join(dir, "workspacer", "config.yaml")

	c := newConfigService()
	if c.persistBlocked {
		t.Fatal("persistBlocked latched on a genuine first run")
	}
	seeded, err := os.ReadFile(p)
	if err != nil {
		t.Fatalf("first run did not seed config.yaml — the wipe guard swallowed the seed: %v", err)
	}
	if !strings.Contains(string(seeded), "keybindings:") {
		t.Fatalf("seeded config.yaml is not the defaults document:\n%s", string(seeded))
	}

	c.save(map[string]any{"ui": map[string]any{"theme": "mytheme"}})
	after, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(after), "mytheme") {
		t.Fatalf("a save after a first-run seed did not reach disk:\n%s", string(after))
	}
}

// TestWriteConfigYAMLAllowsARewriteOfAConfigThatIsAlreadyDefaults keeps the
// refusal from latching a config.yaml that legitimately holds the defaults — a
// fresh install's file. Rewriting defaults over defaults loses nothing, and
// refusing it would block the read-time migrations on a brand-new machine.
func TestWriteConfigYAMLAllowsARewriteOfAConfigThatIsAlreadyDefaults(t *testing.T) {
	dir := tempConfigHome(t)
	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	bare, err := yaml.Marshal(defaultConfig())
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, bare, 0o644); err != nil {
		t.Fatal(err)
	}

	if err := writeConfigYAML(defaultConfig()); err != nil {
		t.Fatalf("writeConfigYAML refused a no-op rewrite of a config that is already the defaults: %v", err)
	}
}

// TestWriteConfigYAMLAllowsAnOrdinarySave proves the guard is inert on the path
// every real save takes: a merge over a populated config is not the bare
// defaults document, so nothing is refused.
func TestWriteConfigYAMLAllowsAnOrdinarySave(t *testing.T) {
	p := seedConfig(t, realisticConfig)

	c := newConfigService()
	c.save(map[string]any{"ui": map[string]any{"theme": "nord"}})

	after, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	var parsed map[string]any
	if err := yaml.Unmarshal(after, &parsed); err != nil {
		t.Fatal(err)
	}
	ui, _ := parsed["ui"].(map[string]any)
	if ui["theme"] != "nord" {
		t.Fatalf("an ordinary save did not reach disk — the wipe guard is over-firing: ui.theme = %v", ui["theme"])
	}
	if cl, _ := parsed["claude"].(map[string]any); cl["skipPermissionsDefault"] != true {
		t.Fatalf("an ordinary save lost an unrelated setting: claude.skipPermissionsDefault = %v", cl["skipPermissionsDefault"])
	}
}

// TestBrainTestsCannotAddressTheRealConfigDir is the durable half. The per-write
// refusal above stops the one destructive shape; this stops the whole class, by
// asserting the test binary's configDir() does not point anywhere near the
// machine's real config. 47 tests in this package build a configService
// (newRegistry does it implicitly) with no redirect of their own — the
// package-level sandbox in TestMain is the only thing standing between them and
// ~/.config/workspacer, and a TestMain that quietly stops setting it would put
// them straight back there with nothing failing.
func TestBrainTestsCannotAddressTheRealConfigDir(t *testing.T) {
	got := configDir()
	if got == "" {
		t.Fatal("configDir() is empty — tests would resolve config paths unpredictably")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		t.Skipf("no home directory to compare against: %v", err)
	}
	real := filepath.Join(home, ".config", "workspacer")
	if got == real {
		t.Fatalf("the cmd/brain test binary resolves configDir() to the DEVELOPER'S OWN %s — "+
			"any test that writes config there destroys the machine's real settings. "+
			"TestMain must sandbox XDG_CONFIG_HOME/APPDATA before m.Run.", real)
	}
}
