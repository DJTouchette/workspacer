package main

// File-backed catalogs: layout templates and saved workspace sessions. Both are
// one-YAML-file-per-item stores under ~/.config/workspacer (layouts/, sessions/),
// ports of layoutService.ts and sessionService.ts. Kept generic (map[string]any)
// so arbitrary pane/tab/agent shapes round-trip without a matching Go type.

import (
	"fmt"
	"log"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	yaml "gopkg.in/yaml.v3"
)

// nowISO matches JS `new Date().toISOString()` (UTC, millisecond precision).
func nowISO() string { return time.Now().UTC().Format("2006-01-02T15:04:05.000Z") }

// storeEntryPath resolves ONE os.ReadDir entry against the store directory it
// came from and refuses anything that does not stay inside it. It returns the
// canonical path, which is the string the caller must then open (BINDING
// DECISION 2).
//
// The entry name is a bare basename, so the join cannot escape textually — but a
// SYMLINK named "x.yaml" sitting in the store is a perfectly legal entry, and
// os.ReadFile follows it. <configDir>/layouts and <configDir>/sessions are the
// two directories a bus caller can write into (they are configStoreRoots), so
// planting that symlink is an ordinary allowed fs.write. Without this the
// listers read straight through it, and quarantineUnreadable then COPIED the
// bytes to <name>.broken-<ts> — a plain file inside the same carve-out, which
// the secret gate permits — so `layouts.list` followed by `fs.read` of the
// quarantine copy handed a caller <configDir>/remote-token, i.e. promotion to a
// TRUSTED bus connection. Same rule as readLibraryDir's per-file guard.
func storeEntryPath(dir, name string) (string, bool) {
	canonical, err := canonicalizePath(filepath.Join(dir, name))
	if err != nil {
		return "", false // unverifiable → skip, same posture as the fs.* guard
	}
	if !isWithin(canonical, dir) {
		return "", false
	}
	return canonical, true
}

func str(v any) string           { s, _ := v.(string); return s }
func asMap(v any) map[string]any { m, _ := v.(map[string]any); return m }
func asSlice(v any) []any        { s, _ := v.([]any); return s }

// ── Layout templates (<configDir>/layouts/<id>.yaml) ────────────────────────

func layoutsDir() string { return filepath.Join(configDir(), "layouts") }

func listLayouts() []map[string]any {
	out := []map[string]any{}
	_ = os.MkdirAll(layoutsDir(), 0o755)
	entries, err := os.ReadDir(layoutsDir())
	if err != nil {
		return out
	}
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".yaml") {
			continue
		}
		full, ok := storeEntryPath(layoutsDir(), e.Name())
		if !ok {
			log.Printf("brain: layout %s does not resolve inside the layouts dir (skipped)", e.Name())
			continue
		}
		data, err := os.ReadFile(full)
		if err != nil {
			log.Printf("brain: could not read layout %s: %v (skipped)", e.Name(), err)
			continue
		}
		var l map[string]any
		if yaml.Unmarshal(data, &l) != nil {
			quarantineUnreadable(full, data)
			continue
		}
		// Match the app: only well-formed layouts (an agents array) are listed.
		if _, ok := l["agents"].([]any); !ok {
			continue
		}
		out = append(out, l)
	}
	sort.SliceStable(out, func(i, j int) bool {
		return str(out[i]["createdAt"]) > str(out[j]["createdAt"]) // newest first
	})
	return out
}

// layoutFilePath resolves the file for a layout id, slugging it first and then
// re-asserting the result stayed inside layoutsDir — the same hazard
// sessionFilePath documents below, but worse: save used the RAW id, so an id of
// "../config" wrote over ~/.config/workspacer/config.yaml, and because the
// clobbered file still parses there is no .broken-* backup to recover the user's
// themes/keybindings/budgets from. An id carrying a separator or a ".." segment
// is an escape attempt rather than an untidy name, so it is refused instead of
// quietly repointed at some other file; everything else is slugged, which is
// also what removeLayout has always done (save and remove disagreed on the
// filename for any id that wasn't already a slug).
func layoutFilePath(id string) (string, error) {
	if strings.ContainsAny(id, `/\`) || strings.Contains(id, "..") {
		return "", fmt.Errorf("layout id must not contain a path separator")
	}
	full := filepath.Join(layoutsDir(), slugLayout(id)+".yaml")
	if filepath.Dir(full) != filepath.Clean(layoutsDir()) {
		return "", fmt.Errorf("layout id resolves outside the layouts directory")
	}
	return full, nil
}

// saveLayout writes one layout file, mirroring layoutService.save: id defaults to
// the slug of the name, name falls back to the id, createdAt is stamped now.
func saveLayout(input map[string]any) (map[string]any, error) {
	id := str(input["id"])
	name := strings.TrimSpace(str(input["name"]))
	if id == "" {
		id = slugLayout(str(input["name"]))
	}
	path, err := layoutFilePath(id)
	if err != nil {
		return nil, err
	}
	// The stored id is the slug we actually wrote under, so a later remove() —
	// which re-slugs — finds this file.
	id = slugLayout(id)
	if name == "" {
		name = id
	}
	agents := input["agents"]
	if agents == nil {
		agents = []any{}
	}
	layout := map[string]any{"id": id, "name": name, "createdAt": nowISO(), "agents": agents}
	if err := os.MkdirAll(layoutsDir(), 0o755); err != nil {
		return nil, err
	}
	data, err := yaml.Marshal(layout)
	if err != nil {
		return nil, err
	}
	if err := writeFileAtomic(path, data, 0o644); err != nil {
		return nil, err
	}
	return layout, nil
}

func removeLayout(id string) {
	path, err := layoutFilePath(id)
	if err != nil {
		return
	}
	_ = os.Remove(path)
}

// ── Saved sessions (<configDir>/sessions/<slug(name)>.yaml) ──────────────────

func sessionsDir() string { return filepath.Join(configDir(), "sessions") }

// sessionSchemaVersion is the saved-session format version this build writes.
// The TS twin is main/shared/sessionSchema.ts; both are pinned to
// contracts/session-schema.json by a test on each side. A reader accepts an
// absent version (pre-versioning) or one <= its own; a HIGHER version means a
// newer build wrote the file and it must not be overwritten.
const sessionSchemaVersion = 1

type sessionListEntry struct {
	Name       string `json:"name"`
	Filename   string `json:"filename"`
	Timestamp  string `json:"timestamp"`
	PaneCount  int    `json:"paneCount"`
	AgentCount int    `json:"agentCount"`
}

// resolveSessionFilename picks the file a session of this name should be written
// to, mirroring sessionService.saveSession's identity check.
//
// Two distinct names can slug to the same file ("Feature: Auth" and
// "Feature Auth" both give feature-auth.yaml). Writing blindly would let the
// second session clobber the first. Reuse a file only when it already holds THIS
// session — which is what keeps ordinary autosaves stable — and otherwise take
// the next free numeric suffix.
//
// A file we cannot read or parse is deliberately treated as "not ours": we will
// not overwrite data we cannot identify. The brain is the default writer under
// DELEGATE_CATALOG_TO_BRAIN, so before this the desktop's copy of this guard was
// the one that never ran.
func resolveSessionFilename(name string) string {
	base := slugSession(name)
	filename := base + ".yaml"
	for i := 2; ; i++ {
		path, ok := sessionFilePath(filename)
		if !ok {
			return base + ".yaml" // containment rejects it; let the caller fail
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			return filename // free slot (or unreadable dir) — take it
		}
		var existing map[string]any
		if yaml.Unmarshal(raw, &existing) == nil && str(existing["name"]) == name {
			return filename // already ours
		}
		filename = fmt.Sprintf("%s-%d.yaml", base, i)
	}
}

// quarantineUnreadable copies a store file we could not parse to a timestamped
// .broken-* sibling, once, so the data survives whatever overwrites it next.
//
// Both listers below skip a file they cannot parse, which means a corrupt
// default.yaml simply vanishes from the list — the app then boots with an empty
// roster and the autosave writes over it. The desktop blocks saving when a
// restore fails, but a file that never appears in the list produces no failure
// to notice, so the copy is the backstop. Mirrors the config loader's .broken-*.
//
// `path` is the CANONICAL store path — storeEntryPath already refused anything
// that resolved out of the store — so the backup lands in that same store
// directory. That matters: a copy of an out-of-store file would be a laundering
// primitive, because the copy is an ordinary readable file inside a carve-out
// the secret gate permits, while the file it was copied FROM is not.
func quarantineUnreadable(path string, data []byte) {
	backup := path + ".broken-" + time.Now().UTC().Format("2006-01-02T15-04-05.000")
	// Only the first sighting writes a copy; a list call happens often and each
	// one must not mint another backup of the same bad file.
	if matches, _ := filepath.Glob(path + ".broken-*"); len(matches) > 0 {
		return
	}
	if err := os.WriteFile(backup, data, 0o644); err != nil {
		log.Printf("brain: could not quarantine unreadable %s: %v", path, err)
		return
	}
	log.Printf("brain: %s could not be parsed; copied to %s and skipped", path, backup)
}

func listSavedSessions() []sessionListEntry {
	out := []sessionListEntry{}
	_ = os.MkdirAll(sessionsDir(), 0o755)
	entries, err := os.ReadDir(sessionsDir())
	if err != nil {
		return out
	}
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".yaml") {
			continue
		}
		full, ok := storeEntryPath(sessionsDir(), e.Name())
		if !ok {
			log.Printf("brain: session %s does not resolve inside the sessions dir (skipped)", e.Name())
			continue
		}
		data, err := os.ReadFile(full)
		if err != nil {
			log.Printf("brain: could not read session %s: %v (skipped)", e.Name(), err)
			continue
		}
		var s map[string]any
		if yaml.Unmarshal(data, &s) != nil {
			// Skipping silently is what lets a corrupt default.yaml disappear
			// from the list and then be overwritten by an empty autosave.
			quarantineUnreadable(full, data)
			continue
		}
		name := str(s["name"])
		if name == "" {
			name = strings.TrimSuffix(e.Name(), ".yaml")
		}
		out = append(out, sessionListEntry{
			Name:       name,
			Filename:   e.Name(),
			Timestamp:  str(s["timestamp"]),
			PaneCount:  paneCount(s),
			AgentCount: agentCount(s),
		})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Timestamp > out[j].Timestamp })
	return out
}

// sessionFilePath resolves a caller-supplied session filename to the file to
// open, or ok=false. It is a FOURTH copy of path containment (the desktop's
// sessionService.resolveWithinSessionsDir is the twin) and it owes the same two
// rules the shared corpus pins under `sessionFilenames`:
//
//  1. A PLAIN BASE NAME, never a path. Anything with a separator, or "." / "..",
//     is refused rather than resolved — filepath.Join runs Clean, which
//     collapses ".." instead of blocking it. (The desktop copy used to accept
//     any multi-segment name that textually landed under the sessions dir, so
//     the two providers disagreed about what a legal session file even is.)
//  2. CANONICALIZE, then contain. A basename is not enough on its own: a
//     symlink in the sessions dir named "x.yaml" is a legal basename and points
//     wherever it likes, and os.ReadFile/os.Remove follow it. So resolve per
//     component and require the RESULT to sit inside sessionsDir — and hand
//     that result back, so the checked path and the opened path are one string
//     (BINDING DECISION 2).
func sessionFilePath(filename string) (string, bool) {
	if filename == "" || filename == "." || filename == ".." || filename != filepath.Base(filename) {
		return "", false
	}
	canonical, err := canonicalizePath(filepath.Join(sessionsDir(), filename))
	if err != nil {
		return "", false // unverifiable → deny, same posture as the fs.* guard
	}
	if !isWithin(canonical, sessionsDir()) {
		return "", false
	}
	return canonical, true
}

func loadSavedSession(filename string) map[string]any {
	path, ok := sessionFilePath(filename)
	if !ok {
		return nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil
	}
	var s map[string]any
	if yaml.Unmarshal(data, &s) != nil {
		return nil
	}
	return s
}

// saveSavedSession persists a session blob and returns its filename. The caller
// has already shaped `data` (name/timestamp/agents|tabs); we only choose the
// filename, matching sessionService.saveSession.
func saveSavedSession(name string, data map[string]any) (string, error) {
	filename := resolveSessionFilename(name)
	// The slug can't produce a separator, but the write goes through the same
	// containment check as the reads so the three paths can never disagree about
	// what a legal session file is.
	path, ok := sessionFilePath(filename)
	if !ok {
		return "", fmt.Errorf("invalid session name")
	}
	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		return "", err
	}
	// Stamp the format version so a future build can tell "I don't understand
	// this" from "this is empty" — see contracts/session-schema.json. Copy so
	// the caller's map is untouched.
	stamped := make(map[string]any, len(data)+1)
	for k, v := range data {
		stamped[k] = v
	}
	stamped["schemaVersion"] = sessionSchemaVersion
	raw, err := yaml.Marshal(stamped)
	if err != nil {
		return "", err
	}
	if err := writeFileAtomic(path, raw, 0o644); err != nil {
		return "", err
	}
	return filename, nil
}

func deleteSavedSession(filename string) {
	path, ok := sessionFilePath(filename)
	if !ok {
		return
	}
	_ = os.Remove(path)
}

// paneCount mirrors sessionService.listSessions: agent-centric panes if present,
// else legacy tabs' panes, else top-level panes.
func paneCount(s map[string]any) int {
	if agents := asSlice(s["agents"]); len(agents) > 0 {
		n := 0
		for _, a := range agents {
			for _, t := range asSlice(asMap(a)["tabs"]) {
				n += len(asSlice(asMap(t)["panes"]))
			}
		}
		return n
	}
	if _, ok := s["tabs"]; ok {
		n := 0
		for _, t := range asSlice(s["tabs"]) {
			n += len(asSlice(asMap(t)["panes"]))
		}
		return n
	}
	return len(asSlice(s["panes"]))
}

// agentCount counts non-global agents, matching the app.
func agentCount(s map[string]any) int {
	n := 0
	for _, a := range asSlice(s["agents"]) {
		if g, _ := asMap(a)["global"].(bool); !g {
			n++
		}
	}
	return n
}
