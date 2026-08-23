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
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
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
		// Never persist a "__proto__" key. The TS twin (configService.ts
		// deepMerge) drops it structurally — `result["__proto__"] = …` reassigns
		// the object's prototype in JS rather than adding an own key, so it never
		// reaches config.yaml — while a Go map keeps "__proto__" as an ordinary
		// key and yaml.Marshal would write it. That divergence let a caller who
		// saves config through the brain plant an attacker-named key the desktop
		// silently strips on the next read: same input, different file on disk.
		// Dropping it here is the safe direction (the key is never anything the
		// schema reads) and makes the two writers agree. Pinned by
		// contracts/deepmerge-cases.json.
		if k == "__proto__" {
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
	loadedAt string // configStamp() of config.yaml when `current` was loaded
	// persistBlocked is set when config.yaml exists but couldn't be loaded
	// (unreadable or unparseable). While set, save() keeps changes in memory
	// only and refuses to write, so one save never overwrites a recoverable
	// user file with defaults+partial. Mirrors configService.persistBlocked.
	persistBlocked bool
	// lastBrokenBackup holds the raw bytes of the last config.yaml written out
	// as a .broken-* backup, so re-reading an unchanged broken file does not
	// mint a fresh backup each time.
	lastBrokenBackup string
}

func newConfigService() *configService {
	c := &configService{}
	c.mu.Lock()
	c.current = c.loadFromDisk()
	c.loadedAt = configStamp()
	c.mu.Unlock()
	return c
}

// configStamp identifies the config file's current contents as "<mtime>:<size>",
// or "" when it's absent (so a missing file never looks like a change against a
// loaded cache).
//
// It carries the SIZE and is compared for INEQUALITY (rather than mtime.After)
// because mtime alone, ordered, cannot see the other writer at all when its save
// lands in the same filesystem timestamp tick — 1s granularity on ext4 with
// 128-byte inodes, HFS+ and NFSv3, 2s on FAT/exFAT. Mirrors the TS twin
// (apps/desktop/src/main/services/configService.ts configStamp).
func configStamp() string {
	st, err := os.Stat(configPath())
	if err != nil {
		return ""
	}
	return fmt.Sprintf("%d:%d", st.ModTime().UnixNano(), st.Size())
}

func (c *configService) loadFromDisk() map[string]any {
	c.persistBlocked = false
	defaults := defaultConfig()
	data, err := os.ReadFile(configPath())
	if err != nil {
		if os.IsNotExist(err) {
			if c.current != nil {
				// We had a config a moment ago — this is a mid-run
				// disappearance (e.g. a hand edit that truncated before
				// rewriting), not a first run. Treating it as first run would
				// seed bare defaults and the next save would write them over
				// whatever the user actually has. Mirrors configService.ts.
				c.persistBlocked = true
				return c.current
			}
			// Genuine first run — no config file yet: seed it with defaults.
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
		// Only back up bytes we have not already backed up: saveLocked re-reads
		// on every save, and a broken file would otherwise mint one .broken-*
		// per save call.
		if string(data) != c.lastBrokenBackup {
			backupPath := configPath() + ".broken-" + time.Now().UTC().Format("2006-01-02T15-04-05.000")
			if err := os.WriteFile(backupPath, data, 0o644); err == nil {
				c.lastBrokenBackup = string(data)
			}
		}
		return defaults
	}
	if parsed == nil {
		// A 0-byte / whitespace-only / comment-only config.yaml unmarshals to a
		// nil map with NO error above — it is not a parse error. deepMerge(defaults,
		// nil) would silently hand back untouched defaults, and saveLocked
		// re-reads via this same function unconditionally, so the next save
		// would write those bare defaults over the user's real config. Treat it
		// like the parse-error branch above: block saves instead. Mirrors
		// configService.ts loadFromDisk.
		c.persistBlocked = true
		return defaults
	}
	c.lastBrokenBackup = ""
	return pruneRemovedShortcuts(migrateFlatChords(migrateKeybindings(deepMerge(defaults, parsed))))
}

func (c *configService) writeDefaults(defaults map[string]any) {
	_ = writeConfig(defaults)
}

// writeConfig is the config writer, indirected through a package variable so the
// write-FAILURE branch of saveLocked is reachable from a test. Its real-world
// triggers (ENOSPC, EIO, EDQUOT, a read-only remount) cannot be produced inside
// a temp directory, and every filesystem trick that breaks the rename also
// breaks the read that now precedes it — which turns the case into the
// persistBlocked branch instead of the one under test.
var writeConfig = writeConfigYAML

func (c *configService) get() map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	// Re-read when the file changed under us (e.g. the desktop app wrote a
	// setting in its own process). mtime-gated, so the steady state is one stat.
	if st := configStamp(); c.current == nil || (st != "" && st != c.loadedAt) {
		c.current = c.loadFromDisk()
		c.loadedAt = configStamp()
	}
	return c.current
}

func (c *configService) reload() map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.current = c.loadFromDisk()
	c.loadedAt = configStamp()
	return c.current
}

// hostTrustedSections are config keys only the host process may write. Every
// config.save this brain answers arrives over the bus — a remote/web client, a
// plugin, or an agent through the MCP facade — so a section listed here can be
// dropped unconditionally: if it came through us, it did not come from the host.
//
// `updates` earns its place: updates.channel is concatenated into the
// electron-updater feed URL the desktop then downloads and installs from, so one
// "../" in a channel relocates the updater to somebody else's repo. That is
// persistent code execution laundered through the app's own update dialog — not
// a setting a bus caller gets to choose. The desktop's own Settings write goes
// through configService.ts in-process and is unaffected.
//
// `scripts` is a map of agent cwd → [{name, command}], which the desktop renders
// as buttons in its top bar and runs by opening a terminal whose initialCommand
// is the string verbatim (App.tsx handleRunScript). The attacker picks the
// button's LABEL too, and the cwd key comes free from the read-only agents.list
// capability. Section-level and not sub-key, because every key under it is a
// caller-chosen directory — there is no fixed sub-key to name.
//
// THE RULE, so the next key gets classified rather than missed: a config value is
// host-trusted when the HOST later hands it to a process — as argv[0], as an argv
// element, or as a line typed into a shell — on a path that does not scrub it.
var hostTrustedSections = []string{"updates", "scripts"}

// hostTrustedPaths are the same rule at SUB-KEY granularity, dotted from the
// root. A section-level drop is the wrong tool for these: they live inside
// sections a bus client legitimately edits, so dropping the whole section would
// break ordinary settings while keeping the section would leave these writable.
//
//   - agents.binaries is the launcher path this brain hands to claudemon as
//     argv[0] for EVERY spawned agent (resolveSpawnBin → spawnReq.Bin →
//     Command::new). config.save is not in capspec.PathParam and its name is not
//     path-bearing, so neither classification detector could see it — while
//     fs.write of config.yaml is refused by the secret gate in all three
//     containment copies, config.save rewrote the same file by design. Combined
//     with an fs.write over an existing executable inside the caller's own agent
//     cwd (os.WriteFile preserves the 0755 mode), that was arbitrary host code
//     execution on the next spawn.
//   - claude.profiles carries configDir (which becomes CLAUDE_CONFIG_DIR, i.e.
//     the settings.json supplying permissions.allow and hooks) and extraArgs
//     (--dangerously-skip-permissions). A bus caller planting one there is
//     persistent, and the desktop's LOCAL spawn path does not scrub it.
//   - terminal.shell and terminal.shells[].path are argv[0] of the next terminal
//     the LOCAL user opens: TerminalPane passes `shell || termCfg.shell` to
//     IPC.TERMINAL_CREATE, which spawns `argv: [resolvedShell]`, and the NavBar
//     "+" menu passes shells[].path the same way. The BUS door onto that
//     primitive (terminals.create) already has a whole shell allow-list
//     (shellallow.go) built on "shell is argv[0] of a process spawned on the
//     host, taken verbatim from a bus caller"; the local IPC door deliberately
//     has none, so config.save must not be a second, PERSISTENT way in. The
//     shipped default panes include three terminals, so it fires on the next
//     launch without the user choosing anything.
//   - editor.terminalCommand is not even argv[0] — it is raw shell TEXT.
//     ScrollContainer builds "<cmd> <file>" and TerminalPane types it into the
//     user's own shell with a trailing CR, so ';' and '|' need no planted binary.
//     Live when editor.engine is "terminal", which the same call can set.
var hostTrustedPaths = []string{
	"agents.binaries",
	"claude.profiles",
	"terminal.shell",
	"terminal.shells",
	"editor.terminalCommand",
}

// dropHostTrusted returns partial without any host-trusted section, leaving the
// on-disk values alone. It copies rather than deletes in place: the caller still
// owns the map it passed us.
func dropHostTrusted(partial map[string]any) map[string]any {
	var found []string
	for _, k := range hostTrustedSections {
		if _, ok := partial[k]; ok {
			found = append(found, k)
		}
	}
	type nested struct{ section, key string }
	var foundPaths []nested
	for _, dotted := range hostTrustedPaths {
		section, key, ok := strings.Cut(dotted, ".")
		if !ok {
			continue
		}
		sub, ok := partial[section].(map[string]any)
		if !ok {
			continue // absent, or not an object — nothing to strip
		}
		if _, ok := sub[key]; ok {
			foundPaths = append(foundPaths, nested{section, key})
		}
	}
	if len(found) == 0 && len(foundPaths) == 0 {
		return partial
	}
	if len(found) > 0 {
		log.Printf("brain: config.save: ignoring host-trusted section(s) %v from a bus client", found)
	}
	out := make(map[string]any, len(partial))
	for k, v := range partial {
		out[k] = v
	}
	for _, k := range found {
		delete(out, k)
	}
	for _, n := range foundPaths {
		log.Printf("brain: config.save: ignoring host-trusted key %s.%s from a bus client", n.section, n.key)
		// Copy the section too: the caller still owns the map it passed us, and
		// the nested map is shared with it.
		sub := out[n.section].(map[string]any)
		cp := make(map[string]any, len(sub))
		for k, v := range sub {
			cp[k] = v
		}
		delete(cp, n.key)
		out[n.section] = cp
	}
	return out
}

func (c *configService) save(partial map[string]any) map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	// The mutex above only serialises US. config.yaml has a second writer in
	// another process, and the refresh → merge → write below is exactly the
	// sequence that must not interleave with theirs: the mtime gate closes the
	// refresh, nothing spans the three. Hold the cross-process lock across all
	// of it. See contracts/config-lock.json.
	var result map[string]any
	if err := withConfigLock(configPath(), func() error {
		result = c.saveLocked(partial)
		return nil
	}); err != nil {
		// Could not take the lock: the other writer is mid-write (or wedged).
		// Writing anyway is the bug this exists to prevent, so refuse and let the
		// caller see the value it did not get.
		// Deliberately does NOT set persistBlocked. That flag means "config.yaml
		// exists but could not be LOADED, so never write over a file we might be
		// able to recover", and it is a one-way latch: saveLocked's
		// persistBlocked branch returns before writeConfigYAML, so the clear at
		// the bottom becomes unreachable and only an external mtime bump can lift
		// it — which cannot happen once we have stopped writing. A lock timeout
		// is transient (an orphaned lockfile is stolen after staleMs), so
		// latching on it turned a ten-second obstruction into a permanently
		// write-only daemon that still reported every save as applied. The TS
		// twin does not latch here either.
		log.Printf("brain: config.save skipped: %v", err)
		return c.current
	}
	return result
}

// wholesaleConfigPaths are the config subtrees replaced WHOLESALE on save
// instead of deep-merged — dotted paths matching the TS twin's
// WHOLESALE_CONFIG_PATHS (apps/desktop/src/main/shared/configWholesale.ts),
// pinned together by contracts/wholesale-config-paths.json. See that fixture
// for the full rationale; in short, each of these is a user-owned map whose
// keys the user can individually delete, and deep-merge can only ever add or
// overwrite a key — so under it a deleted entry comes straight back and the
// caller's only way to express "delete" is to resend the whole surviving map.
//
// This list used to be missing `projects` entirely: the brain hand-special-
// cased only ui.customThemes and claude.budgets, so a project-delete sent over
// MCP save_config (which always answers through the brain, never the
// desktop's in-process path) silently failed to delete. Safe — nothing was
// lost — but a real disagreement between the two config.yaml writers about
// what a save clears, which is exactly the class of bug the TS-side unification
// (commit c4963a73) fixed on only one side.
var wholesaleConfigPaths = []string{"ui.customThemes", "claude.budgets", "projects"}

// applyWholesale replaces merged's value at dottedPath with partial's value at
// the same path, when partial actually names it — undoing whatever deepMerge
// did there. Mirrors configService.ts's applyWholesale exactly (including its
// "absent means untouched, present-but-not-a-map means empty" rule), so the
// two languages cannot answer "was this key touched?" differently.
func applyWholesale(merged map[string]any, partial map[string]any, dottedPath string) {
	keys := strings.Split(dottedPath, ".")
	leaf := keys[len(keys)-1]
	parents := keys[:len(keys)-1]

	src := partial
	dst := merged
	for _, k := range parents {
		nextSrc, ok := src[k].(map[string]any)
		if !ok {
			return // partial doesn't reach this path — not touched
		}
		src = nextSrc
		nextDst, ok := dst[k].(map[string]any)
		if !ok {
			return // the merge never created this parent as an object
		}
		dst = nextDst
	}
	v, present := src[leaf]
	if !present {
		return
	}
	if vMap, ok := v.(map[string]any); ok {
		dst[leaf] = vMap
	} else {
		dst[leaf] = map[string]any{}
	}
}

// preWriteHook runs once per saveLocked attempt, immediately after the merge
// is computed and before the CAS check. A no-op in production; tests
// override it to inject a write that lands in exactly that window (a
// non-lock-participating writer beating us to disk), the same way a real one
// would — see TestSaveCASRetriesAgainstAConcurrentWriter.
var preWriteHook = func() {}

// saveCASAttempts bounds the compare-and-swap retry below. Mirrors
// briefService.ts's CAS_ATTEMPTS (the same "an outside writer beat us,
// recompute against what's actually there" shape) — a save is rare enough
// that a few retries cost nothing, and giving up loudly beats writing over a
// change we cannot see.
const saveCASAttempts = 5

// saveLocked is save's body, run while holding both the in-process mutex and
// the cross-process config lock (config.yaml.lock).
//
// The lock alone is enough for the two COOPERATING writers (this brain and
// the desktop's configService, both of which take it): between them, nothing
// can land between our read and our write. It says nothing about a writer
// that does not participate — a hand edit of config.yaml, or (inside THIS
// process) loadFromDisk's own one-time keybindings/chord/shortcut migrations,
// which call writeConfigYAML directly rather than routing back through
// save(). Those are the only remaining window, and CAS is how brief_append
// closes the equivalent one for brief.md: re-check immediately before
// publishing, and if the file moved under us, recompute against what is
// actually there instead of overwriting it.
func (c *configService) saveLocked(partial map[string]any) map[string]any {
	dropped := dropHostTrusted(partial)
	var merged map[string]any
	for attempt := 0; attempt < saveCASAttempts; attempt++ {
		// Fold in any external write (e.g. the desktop app editing config.yaml in
		// its own process) before merging our partial, so a stale cache doesn't
		// clobber it. UNCONDITIONALLY, not through the stamp gate: the gate is a
		// cheap-read optimisation for get(), and a write we cannot see is the
		// exact failure it would let through (the other writer's save landing in
		// the same filesystem tick at the same length). Mirrors the TS twin
		// (configService.ts saveConfigLocked).
		c.current = c.loadFromDisk()
		c.loadedAt = configStamp()
		merged = deepMerge(c.current, dropped)
		for _, dotted := range wholesaleConfigPaths {
			applyWholesale(merged, dropped, dotted)
		}
		if c.persistBlocked {
			// The on-disk config failed to load (unreadable or unparseable): keep
			// the change in memory only. Writing here would replace the user's file
			// with defaults + this partial — permanent loss of everything else.
			c.current = merged
			return merged
		}
		preWriteHook()
		// COMPARE-AND-SWAP: re-check the file's identity immediately before
		// publishing. A non-lock-participating writer does not block on
		// withConfigLock, so "nobody changed it while we computed the merge" is a
		// claim that has to be checked, not assumed. Same stamp (mtime:size,
		// compared for inequality) get() already uses — cheap, and this is a
		// stat, not a re-read, of a file we are about to hold the lock across
		// writing anyway.
		if configStamp() != c.loadedAt {
			continue // moved under us — recompute against the writer that beat us
		}
		if err := writeConfig(merged); err != nil {
			// Do not adopt a value that is not on disk — serving it would make the
			// setting look applied until the next restart reverted it. Also does
			// not latch persistBlocked: ENOSPC and EIO are transient, and the latch
			// is unclearable from here for the reason described in save().
			return c.current
		}
		c.current = merged
		c.loadedAt = configStamp()
		return merged
	}
	// Exhausted retries: something outside the lock is rewriting config.yaml
	// faster than a save can land. Refuse rather than write over whatever it
	// left — the caller sees the value it did not get, same as a lock timeout.
	log.Printf("brain: config.save: config.yaml is being rewritten outside the lock faster than "+
		"this save could land (%d attempts) — nothing written", saveCASAttempts)
	return c.current
}

func (c *configService) path() string { return configPath() }

// writeConfigYAML persists the whole config. Returns the write error rather than
// swallowing it: a discarded error here reported a SUCCESSFUL config.save for a
// file that never reached disk, and the in-memory copy then served the phantom
// value for the rest of the process's life — the setting looked applied until
// the next restart silently reverted it.
func writeConfigYAML(cfg map[string]any) error {
	dir := configDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		log.Printf("brain: config dir unavailable: %v", err)
		return err
	}
	data, err := yaml.Marshal(cfg)
	if err != nil {
		log.Printf("brain: config marshal failed: %v", err)
		return err
	}
	if err := writeFileAtomic(configPath(), data, 0o644); err != nil {
		log.Printf("brain: config write failed: %v", err)
		return err
	}
	return nil
}

// writeFileAtomic replaces path with data via a unique temp file in the SAME
// directory + a rename over the target. A rename within one filesystem is
// atomic, so a crash/power-loss mid-write or a concurrent reader sees either the
// old, complete file or the new one — never a half-written one that the reader
// treats as a parse error (and, for config.yaml, backs up as .broken-*).
//
// This is the brain's atomicWriteFileSync: every file-backed store here is
// brain-delegated by default, so the desktop twin's atomic write is the one that
// never runs. Sharing one helper is what keeps the two from drifting apart again
// the next time a store is added.
func writeFileAtomic(path string, data []byte, perm os.FileMode) error {
	tmp, err := os.CreateTemp(filepath.Dir(path), "."+filepath.Base(path)+".tmp-*")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		_ = os.Remove(tmpName)
		return err
	}
	// Push the contents to disk before the rename publishes them.
	//
	// The rename is atomic against other READERS, but says nothing about
	// durability: after a crash or power loss the directory entry can have
	// reached the disk while the contents are still in the page cache, leaving a
	// correctly-named EMPTY file — worse than the half-written file this helper
	// exists to prevent, because the complete copy it replaced is already gone.
	//
	// Best-effort, matching the desktop twin's fsyncFile: a filesystem that
	// cannot flush should not turn a successful save into a failed one, and the
	// rename below still publishes the data.
	_ = tmp.Sync()
	if err := tmp.Close(); err != nil {
		_ = os.Remove(tmpName)
		return err
	}
	// CreateTemp makes the file 0600; the stores are user-readable config.
	if err := os.Chmod(tmpName, perm); err != nil {
		_ = os.Remove(tmpName)
		return err
	}
	if err := os.Rename(tmpName, path); err != nil {
		_ = os.Remove(tmpName)
		return err
	}
	return nil
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
		_ = writeConfigYAML(cfg)
	}
	return cfg
}

// migrateKeybindings ports configService.migrateKeybindings: the old
// mode/leader scheme (or a missing prefix) is reset to the prefix-forward
// defaults. Idempotent — runs once because the rewrite leaves a valid prefix
// and no mode/leader. (It used to preserve Vim mode as editor.vim; that field
// died with the in-app CodeMirror editor — nothing reads it.)
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
	def := defaultConfig()
	cfg["keybindings"] = def["keybindings"]
	_ = writeConfigYAML(cfg)
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
		_ = writeConfigYAML(cfg)
	}
	return cfg
}
