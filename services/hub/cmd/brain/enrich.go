package main

// Snapshot enrichment for the live session store: overlay a custom name, parent,
// and supervisor flag onto claudemon's raw session snapshot, so a headless
// agents.list matches the desktop's named/nested view.
//
// Two sources, mirroring the app + TUI:
//   - spawn metadata (label / parentSessionId / isSupervisor) recorded when the
//     brain spawns an agent — like claudeSessionStore.setSpawnMeta;
//   - persisted cwd→name renames from ~/.config/workspacer/tui-names.json — the
//     same file the TUI writes, keyed by cwd so a rename survives respawns.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
)

type spawnMeta struct {
	Label           string
	ParentSessionID string
	IsSupervisor    bool
}

// metaStore holds spawn metadata keyed by session id. Populated by the spawn
// handler; read by the enricher. Concurrent-safe (spawns and the store runner
// touch it from different goroutines).
type metaStore struct {
	mu sync.RWMutex
	m  map[string]spawnMeta
}

func newMetaStore() *metaStore { return &metaStore{m: map[string]spawnMeta{}} }

func (s *metaStore) set(id string, meta spawnMeta) {
	s.mu.Lock()
	s.m[id] = meta
	s.mu.Unlock()
}

func (s *metaStore) get(id string) (spawnMeta, bool) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	meta, ok := s.m[id]
	return meta, ok
}

// namesByCwd reads the persisted cwd→name renames. Empty on any problem (names
// are a convenience, never load-bearing — matching the TUI).
func namesByCwd() map[string]string {
	out := map[string]string{}
	data, err := os.ReadFile(filepath.Join(configDir(), "tui-names.json"))
	if err != nil {
		return out
	}
	_ = json.Unmarshal(data, &out)
	return out
}

// enrichSnapshot overlays label / parentSessionId / isSupervisor onto a raw
// claudemon snapshot. A spawn label wins over a cwd rename.
func enrichSnapshot(snap json.RawMessage, meta *metaStore) json.RawMessage {
	var m map[string]any
	if json.Unmarshal(snap, &m) != nil {
		return snap
	}
	id, _ := m["session_id"].(string)
	cwd, _ := m["cwd"].(string)

	if meta != nil && id != "" {
		if sm, ok := meta.get(id); ok {
			if sm.Label != "" {
				m["label"] = sm.Label
			}
			if sm.ParentSessionID != "" {
				m["parentSessionId"] = sm.ParentSessionID
			}
			if sm.IsSupervisor {
				m["isSupervisor"] = true
			}
		}
	}
	if _, hasLabel := m["label"]; !hasLabel && cwd != "" {
		if name := namesByCwd()[cwd]; name != "" {
			m["label"] = name
		}
	}

	out, err := json.Marshal(m)
	if err != nil {
		return snap
	}
	return out
}
