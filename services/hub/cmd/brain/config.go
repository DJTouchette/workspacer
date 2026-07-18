package main

// config.* — the workspacer config document (theme, keybindings, pane settings,
// claude defaults, …). A faithful Go port of configService.ts: the same
// config.yaml at ~/.config/workspacer, the same deep-merge-over-defaults on
// read, the same one-time keybindings migration, so a headless client (web,
// TUI) loads the *real* config instead of falling back to renderer defaults.
//
// Config is kept generic (map[string]any) rather than a typed struct: that
// mirrors the TS deepMerge's object semantics exactly and means a new config key
// added on the app side flows through without a matching Go change.

import (
	_ "embed"
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"time"

	yaml "gopkg.in/yaml.v3"
)

// defaultConfigJSON is the SINGLE SOURCE OF TRUTH for the default config,
// embedded from config_defaults.json (the non-Windows shell list; Windows host
// support can come later). The desktop app consumes the very same file: its
// configDefaults.generated.ts is generated from it by
// apps/desktop/scripts/gen-config-defaults.mjs, and a drift test on each side
// fails the build if they diverge — so the two runtimes can no longer drift the
// way the old hand-transcribed copy did.
//
//go:embed config_defaults.json
var defaultConfigJSON string

func defaultConfig() map[string]any {
	var m map[string]any
	_ = json.Unmarshal([]byte(defaultConfigJSON), &m)
	return m
}

func configPath() string {
	return filepath.Join(configDir(), "config.yaml")
}

// deepMerge overlays source onto a shallow copy of target. A null source value
// means "unset" (skip, keep the default); nested maps recurse; everything else
// (incl. arrays) replaces. Mirrors configService.deepMerge.
func deepMerge(target, source map[string]any) map[string]any {
	result := make(map[string]any, len(target))
	for k, v := range target {
		result[k] = v
	}
	for k, sv := range source {
		if sv == nil {
			continue
		}
		if svMap, ok := sv.(map[string]any); ok {
			if tvMap, ok := result[k].(map[string]any); ok {
				result[k] = deepMerge(tvMap, svMap)
				continue
			}
		}
		result[k] = sv
	}
	return result
}

// configService caches the merged config and serializes access, since bus calls
// are handled on concurrent goroutines. Mirrors the TS singleton — but, unlike a
// single-process app, the desktop writes config.yaml in *its* process, so the
// cache is mtime-gated: get() re-reads when the file changed underneath us, so a
// remote client reading via the brain never sees stale config.
type configService struct {
	mu       sync.Mutex
	current  map[string]any
	loadedAt time.Time // mtime of config.yaml when `current` was loaded
	// persistBlocked is set when config.yaml exists but couldn't be loaded
	// (unreadable or unparseable). While set, save() keeps changes in memory
	// only and refuses to write, so one save never overwrites a recoverable
	// user file with defaults+partial. Mirrors configService.persistBlocked.
	persistBlocked bool
}

func newConfigService() *configService {
	c := &configService{}
	c.mu.Lock()
	c.current = c.loadFromDisk()
	c.loadedAt = configMtime()
	c.mu.Unlock()
	return c
}

// configMtime is the config file's modification time, or the zero time when it's
// absent (so a missing file never looks "newer" than a loaded cache).
func configMtime() time.Time {
	if st, err := os.Stat(configPath()); err == nil {
		return st.ModTime()
	}
	return time.Time{}
}

func (c *configService) loadFromDisk() map[string]any {
	c.persistBlocked = false
	defaults := defaultConfig()
	data, err := os.ReadFile(configPath())
	if err != nil {
		if os.IsNotExist(err) {
			// First run — no config file yet: seed it with defaults.
			c.writeDefaults(defaults)
			return defaults
		}
		// Transient read failure (EACCES, EBUSY, …): the file exists but we
		// couldn't read it. Run on defaults in memory and NEVER write over a
		// file we couldn't read. Mirrors configService.loadFromDisk.
		c.persistBlocked = true
		return defaults
	}
	var parsed map[string]any
	if err := yaml.Unmarshal(data, &parsed); err != nil {
		// Malformed YAML (e.g. a hand-edit left a syntax error). This must NOT
		// wipe the user's config: back the broken file up, block saves so
		// nothing overwrites it, and run on defaults in memory. Mirrors the
		// desktop configService.loadFromDisk.
		c.persistBlocked = true
		backupPath := configPath() + ".broken-" + time.Now().UTC().Format("2006-01-02T15-04-05.000")
		_ = os.WriteFile(backupPath, data, 0o644)
		return defaults
	}
	return pruneRemovedShortcuts(migrateFlatChords(migrateKeybindings(deepMerge(defaults, parsed))))
}

func (c *configService) writeDefaults(defaults map[string]any) {
	writeConfigYAML(defaults)
}

func (c *configService) get() map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	// Re-read when the file changed under us (e.g. the desktop app wrote a
	// setting in its own process). mtime-gated, so the steady state is one stat.
	if c.current == nil || configMtime().After(c.loadedAt) {
		c.current = c.loadFromDisk()
		c.loadedAt = configMtime()
	}
	return c.current
}

func (c *configService) reload() map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.current = c.loadFromDisk()
	c.loadedAt = configMtime()
	return c.current
}

func (c *configService) save(partial map[string]any) map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	// Fold in any external write (e.g. the desktop app editing config.yaml in its
	// own process) before merging our partial, so a stale cache doesn't clobber
	// it. Mirrors the mtime gate in get().
	if c.current == nil || configMtime().After(c.loadedAt) {
		c.current = c.loadFromDisk()
		c.loadedAt = configMtime()
	}
	merged := deepMerge(c.current, partial)
	// ui.customThemes is a map of user-created entries: when the caller sends it,
	// it is the whole truth. Deep-merge would resurrect deleted themes (it never
	// removes keys), so replace it wholesale instead. Mirrors
	// configService.saveConfig.
	if uiPartial, ok := partial["ui"].(map[string]any); ok {
		if ct, present := uiPartial["customThemes"]; present {
			mergedUI, _ := merged["ui"].(map[string]any)
			if mergedUI == nil {
				mergedUI = map[string]any{}
				merged["ui"] = mergedUI
			}
			if ctMap, ok := ct.(map[string]any); ok {
				mergedUI["customThemes"] = ctMap
			} else {
				mergedUI["customThemes"] = map[string]any{}
			}
		}
	}
	if c.persistBlocked {
		// The on-disk config failed to load (unreadable or unparseable): keep
		// the change in memory only. Writing here would replace the user's file
		// with defaults + this partial — permanent loss of everything else.
		c.current = merged
		return merged
	}
	writeConfigYAML(merged)
	c.current = merged
	c.loadedAt = configMtime()
	return merged
}

func (c *configService) path() string { return configPath() }

func writeConfigYAML(cfg map[string]any) {
	dir := configDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return
	}
	data, err := yaml.Marshal(cfg)
	if err != nil {
		return
	}
	// Atomic write: a unique temp file in the SAME dir + rename over the target.
	// A rename within one filesystem is atomic, so a crash/power-loss mid-write or
	// a concurrent reader sees either the old, complete file or the new one —
	// never a half-written config.yaml that loadFromDisk would treat as a parse
	// error and back up as .broken-*. Mirrors the desktop's atomicWriteFileSync;
	// a plain truncating os.WriteFile leaves the file corrupt if interrupted.
	tmp, err := os.CreateTemp(dir, ".config.yaml.tmp-*")
	if err != nil {
		return
	}
	tmpName := tmp.Name()
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		_ = os.Remove(tmpName)
		return
	}
	if err := tmp.Close(); err != nil {
		_ = os.Remove(tmpName)
		return
	}
	_ = os.Chmod(tmpName, 0o644)
	if err := os.Rename(tmpName, configPath()); err != nil {
		_ = os.Remove(tmpName)
	}
}

// removedShortcuts are action ids deleted from the app whose bindings were
// historically persisted to disk (the full shortcuts map used to be written on
// first run, by migrateKeybindings, and by Settings rebinds). Mirrors
// configService.pruneRemovedShortcuts: strip them on read so clients never
// build dead chord-tree leaves, and persist the cleanup.
var removedShortcuts = []string{"cycle-view"}

func pruneRemovedShortcuts(cfg map[string]any) map[string]any {
	kb, _ := cfg["keybindings"].(map[string]any)
	if kb == nil {
		return cfg
	}
	shortcuts, _ := kb["shortcuts"].(map[string]any)
	if shortcuts == nil {
		return cfg
	}
	changed := false
	for _, action := range removedShortcuts {
		if _, ok := shortcuts[action]; ok {
			delete(shortcuts, action)
			changed = true
		}
	}
	if changed {
		writeConfigYAML(cfg)
	}
	return cfg
}

// migrateKeybindings ports configService.migrateKeybindings: the old
// mode/leader scheme (or a missing prefix) is reset to the prefix-forward
// defaults, preserving Vim mode as editor.vim. Idempotent — runs once because
// the rewrite leaves a valid prefix and no mode/leader.
func migrateKeybindings(cfg map[string]any) map[string]any {
	kb, _ := cfg["keybindings"].(map[string]any)
	if kb == nil {
		return cfg
	}
	_, hasMode := kb["mode"]
	_, hasLeader := kb["leader"]
	prefix, _ := kb["prefix"].(string)
	legacy := hasMode || hasLeader || prefix == ""
	if !legacy {
		return cfg
	}
	hadVim := false
	if m, ok := kb["mode"].(string); ok && m == "vim" {
		hadVim = true
	}
	def := defaultConfig()
	cfg["keybindings"] = def["keybindings"]
	if hadVim {
		ed, _ := cfg["editor"].(map[string]any)
		if ed == nil {
			ed = map[string]any{}
		}
		ed["vim"] = true
		cfg["editor"] = ed
	}
	writeConfigYAML(cfg)
	return cfg
}

// oldChordDefaults are the pre-flattening nested chord defaults. A saved
// shortcut still holding one of these exact values was never customized by the
// user — it's a stale default — so migrateFlatChords rewrites it to the current
// flat default. Mirrors configService.OLD_CHORD_DEFAULTS.
var oldChordDefaults = map[string]string{
	"new-terminal":   "prefix n t",
	"new-claude":     "prefix n c",
	"new-browser":    "prefix n b",
	"prev-tab":       "prefix t [",
	"next-tab":       "prefix t ]",
	"move-tab-left":  "prefix t ,",
	"move-tab-right": "prefix t .",
	"rename-tab":     "prefix t r",
	"close-pane":     "prefix t w",
	"split":          "prefix p s",
	"quick-split":    "prefix p c",
	"nav-left":       "prefix p h",
	"nav-down":       "prefix p j",
	"nav-up":         "prefix p k",
	"nav-right":      "prefix p l",
}

// migrateFlatChords ports configService.migrateFlatChords: a second-pass
// migration for configs that postdate the schema rewrite (so migrateKeybindings
// leaves them alone) but predate the chord flattening. Any shortcut still equal
// to its old nested default is rewritten to the current flat default; a value
// that differs is a real user choice and is kept. Idempotent.
func migrateFlatChords(cfg map[string]any) map[string]any {
	kb, _ := cfg["keybindings"].(map[string]any)
	if kb == nil {
		return cfg
	}
	shortcuts, _ := kb["shortcuts"].(map[string]any)
	if shortcuts == nil {
		return cfg
	}
	def := defaultConfig()
	defKb, _ := def["keybindings"].(map[string]any)
	defShortcuts, _ := defKb["shortcuts"].(map[string]any)
	changed := false
	for action, oldDefault := range oldChordDefaults {
		cur, ok := shortcuts[action].(string)
		if !ok || cur != oldDefault {
			continue
		}
		if newDefault, ok := defShortcuts[action].(string); ok {
			shortcuts[action] = newDefault
			changed = true
		}
	}
	if changed {
		writeConfigYAML(cfg)
	}
	return cfg
}
