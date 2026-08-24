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

// fleetSenderHeader is the "[fleet] session:<id> (<label>) says:\n" prefix
// registry.sendMessage stamps onto a message whose caller named itself (see
// handlers.go). Label comes from the same spawn-metadata source enrichSnapshot
// reads; a session enrichment never recorded a label for is still named by id
// alone rather than going unattributed.
func fleetSenderHeader(meta *metaStore, sessionID string) string {
	label := ""
	if meta != nil {
		if sm, ok := meta.get(sessionID); ok {
			label = sm.Label
		}
	}
	if label != "" {
		return "[fleet] session:" + sessionID + " (" + label + ") says:\n"
	}
	return "[fleet] session:" + sessionID + " says:\n"
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
// lastActivity / usage / pendingApproval / pendingQuestions / statusLine /
// totalToolCalls — onto each row.
// `sparse: true` marks the row as state-only (no conversation) so a client
// already holding a rich desktop snapshot for the session merges the state in
// instead of replacing the whole thing (see mobile.html upsert and
// webBackend.ts foldSparse). TestCompatSnapshotCoversMobileFields guards the
// field list against mobile.html drift.

// compatSnapshot overlays the desktop snapshot field names onto a raw
// claudemon session row. Snake_case originals are kept alongside.
// toolInputOf unwraps a PermissionRequest hook payload down to the tool's own
// arguments, falling back to the whole payload when it carries none. The twin
// is claudeSessionStore.ts's `raw.tool_input ?? raw`.
func toolInputOf(raw any) any {
	if m, ok := raw.(map[string]any); ok {
		if ti, ok := m["tool_input"]; ok {
			return ti
		}
		// Older payloads spelled it `input`; the daemon's approval_input reader
		// accepts both, so this must too.
		if ti, ok := m["input"]; ok {
			return ti
		}
	}
	return raw
}

// enrichAndCompat composes enrichSnapshot + compatSnapshot in the one order
// callers need: label/parentSessionId/isSupervisor overlaid before the
// desktop-shape fields, since compatSnapshot's own overlay doesn't touch
// them. The ONE place both applications happen — the live session store
// (main.go's store.enrich) and the sessions.snapshot fallback for a session
// the store doesn't hold — so a fallback path can no longer drift from the
// main one by calling compatSnapshot alone.
func enrichAndCompat(snap json.RawMessage, meta *metaStore) json.RawMessage {
	return compatSnapshot(enrichSnapshot(snap, meta))
}

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
	// totalToolCalls: claudemon's tool_calls counter, desktop-named.
	if tc, ok := m["tool_calls"]; ok {
		m["totalToolCalls"] = tc
	}
	// statusLine: claudemon's snake_case StatusLine → the desktop's camelCase
	// SessionStatusLine shape (claudeSessionStore.ts). Headless (no-desktop)
	// sessions otherwise carry only the raw `status_line`, so mobile's
	// stats/progressFingerprint/statusLineAlive read 0/undefined for the phone's
	// strongest fingerprint signal — see .rivet/learnings 2026-08-23.
	if sl, ok := m["status_line"].(map[string]any); ok {
		m["statusLine"] = map[string]any{
			"modelDisplay":        sl["model_display"],
			"effort":              sl["effort"],
			"contextUsedPct":      sl["context_used_pct"],
			"contextWindowSize":   sl["context_window_size"],
			"totalInputTokens":    sl["total_input_tokens"],
			"totalOutputTokens":   sl["total_output_tokens"],
			"costUSD":             sl["cost_usd"],
			"fiveHourPct":         sl["five_hour_pct"],
			"fiveHourResetsAt":    sl["five_hour_resets_at"],
			"fiveHourWindowMins":  sl["five_hour_window_minutes"],
			"sevenDayPct":         sl["seven_day_pct"],
			"sevenDayResetsAt":    sl["seven_day_resets_at"],
			"sevenDayWindowMins":  sl["seven_day_window_minutes"],
			"monthlyPct":          sl["monthly_pct"],
			"monthlyResetsAt":     sl["monthly_resets_at"],
			"monthlyWindowMins":   sl["monthly_window_minutes"],
			"rateLimitWarning":    sl["rate_limit_warning"],
			"overageOutOfCredits": sl["overage_out_of_credits"],
			"capabilities":        sl["capabilities"],
			"receivedAt":          sl["received_at"],
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
				"toolName": p["tool"],
				// `raw` is the whole PermissionRequest hook payload
				// (tool_name, session_id, tool_input, …). The approval card wants
				// the tool's ARGUMENTS — mirroring claudeSessionStore's
				// `raw.tool_input ?? raw` — otherwise the headless clients render
				// the envelope as JSON noise where the command should be.
				"toolInput": toolInputOf(p["raw"]),
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
