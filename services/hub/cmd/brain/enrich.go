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
	"time"
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

// ── Desktop-shape compatibility overlay ─────────────────────────────────────
//
// The desktop publishes rich ClaudeSessionSnapshot objects (camelCase, with
// conversation); the brain's store holds claudemon's raw rows (snake_case).
// The mobile client and the web renderer key everything off the desktop field
// names, so overlay the ones they read — sessionId / status / ambientState /
// lastActivity / usage / pendingApproval / pendingQuestions — onto each row.
// `sparse: true` marks the row as state-only (no conversation) so a client
// already holding a rich desktop snapshot for the session merges the state in
// instead of replacing the whole thing (see mobile.html upsert and
// webBackend.ts foldSparse). TestCompatSnapshotCoversMobileFields guards the
// field list against mobile.html drift.

// compatSnapshot overlays the desktop snapshot field names onto a raw
// claudemon session row. Snake_case originals are kept alongside.
func compatSnapshot(snap json.RawMessage) json.RawMessage {
	var m map[string]any
	if json.Unmarshal(snap, &m) != nil {
		return snap
	}
	id, _ := m["session_id"].(string)
	if id == "" {
		return snap // not a claudemon row — leave untouched
	}
	mode, _ := m["mode"].(string)
	m["sessionId"] = id
	m["sparse"] = true
	if mode == "stopped" {
		m["status"] = "ended"
	} else {
		m["status"] = "active"
	}
	m["ambientState"] = ambientForMode(mode)
	if ts, ok := m["updated_at"].(string); ok {
		if t, err := time.Parse(time.RFC3339, ts); err == nil {
			m["lastActivity"] = t.UnixMilli()
		}
	}
	// usage: claudemon's snake_case counters → the desktop's camelCase shape.
	if u, ok := m["usage"].(map[string]any); ok {
		m["usage"] = map[string]any{
			"model":         u["model"],
			"contextTokens": u["context_tokens"],
			"contextLimit":  u["context_limit"],
			"costUSD":       u["cost_usd"],
		}
	}
	// pending → pendingApproval / pendingQuestions. Set both explicitly (null
	// when absent) so a sparse merge clears a stale decision on the client.
	m["pendingApproval"] = nil
	m["pendingQuestions"] = nil
	if p, ok := m["pending"].(map[string]any); ok {
		switch p["kind"] {
		case "approval":
			m["pendingApproval"] = map[string]any{
				"toolName":  p["tool"],
				"toolInput": p["raw"],
			}
		case "question":
			m["pendingQuestions"] = p["questions"]
		}
	}
	out, err := json.Marshal(m)
	if err != nil {
		return snap
	}
	return out
}

// ambientForMode maps claudemon's SessionMode vocabulary onto the desktop's
// SessionAmbientState one (ipcTypes.ts): the two working states collapse to
// streaming; approval/question map to the two waiting states; everything else
// (unknown / input / stopped) reads as idle.
func ambientForMode(mode string) string {
	switch mode {
	case "responding":
		return "streaming"
	case "approval":
		return "waiting_approval"
	case "question":
		return "waiting_input"
	default:
		return "idle"
	}
}
