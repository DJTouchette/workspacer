package main

// File-backed catalogs: layout templates and saved workspace sessions. Both are
// one-YAML-file-per-item stores under ~/.config/workspacer (layouts/, sessions/),
// ports of layoutService.ts and sessionService.ts. Kept generic (map[string]any)
// so arbitrary pane/tab/agent shapes round-trip without a matching Go type.

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	yaml "gopkg.in/yaml.v3"
)

// nowISO matches JS `new Date().toISOString()` (UTC, millisecond precision).
func nowISO() string { return time.Now().UTC().Format("2006-01-02T15:04:05.000Z") }

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
		data, err := os.ReadFile(filepath.Join(layoutsDir(), e.Name()))
		if err != nil {
			continue
		}
		var l map[string]any
		if yaml.Unmarshal(data, &l) != nil {
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

type sessionListEntry struct {
	Name       string `json:"name"`
	Filename   string `json:"filename"`
	Timestamp  string `json:"timestamp"`
	PaneCount  int    `json:"paneCount"`
	AgentCount int    `json:"agentCount"`
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
		data, err := os.ReadFile(filepath.Join(sessionsDir(), e.Name()))
		if err != nil {
			continue
		}
		var s map[string]any
		if yaml.Unmarshal(data, &s) != nil {
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

// sessionFilePath joins filename onto sessionsDir but only for a plain base name
// — anything containing a path separator or "." / ".." is rejected so a caller
// can't escape the sessions directory via traversal (e.g. "../../etc/passwd").
// filepath.Join runs Clean, which collapses ".." rather than blocking it, so the
// raw client-supplied filename must be validated here.
func sessionFilePath(filename string) (string, bool) {
	if filename == "" || filename == "." || filename == ".." || filename != filepath.Base(filename) {
		return "", false
	}
	return filepath.Join(sessionsDir(), filename), true
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
	filename := slugSession(name) + ".yaml"
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
	raw, err := yaml.Marshal(data)
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
