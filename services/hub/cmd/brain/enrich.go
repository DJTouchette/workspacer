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
	// Live background work the MODE deliberately does not carry. claudemon
	// holds "responding" only for an async subagent; a `run_in_background`
	// shell (a dev server, a watcher, an agent-authored poll loop) rides this
	// count instead, because treating those as busy latched sessions
	// "responding" forever (claude_stream.rs's background_tasks_changed says
	// so, and that reasoning stands). The count was on the wire and NOTHING
	// consulted it: this overlay never emitted it under the name clients read,
	// so an agent that left `npm run dev` running reported a flat idle to every
	// headless consumer. Emit it, and fold it into ambientState the same way
	// the desktop's applyManagedMode does.
	bg := backgroundTasksOf(m)
	if bg > 0 {
		m["backgroundTasks"] = bg
	}
	if ambient, ok := ambientForMode(mode); ok {
		// "the turn ended but work it spawned is still running" — the one
		// ambient state that exists precisely so idle stays honest.
		if ambient == "idle" && mode == "input" && bg > 0 {
			ambient = "background"
		}
		m["ambientState"] = ambient
	}
	if ts, ok := m["updated_at"].(string); ok {
		if t, err := time.Parse(time.RFC3339, ts); err == nil {
			m["lastActivity"] = t.UnixMilli()
		}
	}
	// usage: claudemon's snake_case counters → the desktop's camelCase shape.
	if u, ok := m["usage"].(map[string]any); ok {
		usage := map[string]any{
			"model":         u["model"],
			"contextTokens": u["context_tokens"],
			"contextLimit":  u["context_limit"],
			"costUSD":       u["cost_usd"],
		}
		// The fresh/write/read prompt-cache split, when claudemon reported one.
		// Its sub-keys are already the names the desktop uses, so it passes
		// through whole. Set only when present: a key mapped to nil here would
		// tell mobile and web that nothing was cached, which is a different
		// claim from "the provider did not say".
		if c, ok := u["cache"]; ok && c != nil {
			usage["cache"] = c
		}
		m["usage"] = usage
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
			"cachedInputTokens":   sl["cached_input_tokens"],
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

// backgroundTasksOf reads claudemon's `background_tasks` counter off a raw row.
// JSON numbers land as float64; anything else (absent, null, a string) reads 0,
// which is the same "no claim" a pre-field row makes.
func backgroundTasksOf(m map[string]any) int {
	switch v := m["background_tasks"].(type) {
	case float64:
		if v <= 0 {
			return 0
		}
		return int(v)
	case int:
		if v <= 0 {
			return 0
		}
		return v
	}
	return 0
}

// ambientForMode maps claudemon's SessionMode vocabulary onto the desktop's
// SessionAmbientState one (ipcTypes.ts): the working state becomes streaming;
// approval/question map to the two waiting states; input and stopped read as
// idle. The bool is false for a mode this vocabulary CANNOT express, and the
// caller must then emit no ambientState at all.
//
// That second return is the whole point of the function, and it exists because
// the arm it replaced was `default: return "idle"`.
//
// `unknown` is claudemon's #[derive(Default)] variant: no hook and no driver
// event has arrived yet. A session that is SPAWNING sits in it, and so does one
// register_spawn just flipped back from Stopped on a resume, and so does a
// terminal PTY for its whole life (nothing anywhere in workspacer tracks a
// terminal's busy/idle state, so it never leaves). Reporting any of those as
// idle is a claim that the session is finished when it has not started, and on
// the headless path this overlay is the ONLY thing that answers the question:
// /m, the web renderer and anything reading the bus believe it.
//
// The desktop's twin already answers correctly and is the model followed here:
// claudeSessionStore's applyManagedMode returns early on unknown and leaves the
// previous ambientState untouched. An overlay holds no previous state, so the
// equivalent of "leave it alone" is to omit the field — which is also what the
// sparse-merge contract on the other end means by an absent key (mobile.html's
// upsert and webBackend's foldSparse both merge rather than replace), and what
// every renderer that types the field `SessionAmbientState | undefined`
// already handles.
func ambientForMode(mode string) (string, bool) {
	switch mode {
	case "responding":
		return "streaming", true
	case "approval":
		return "waiting_approval", true
	case "question":
		return "waiting_input", true
	case "input":
		return "idle", true
	case "stopped":
		// An ended row: `status` already says so, and idle is the honest
		// reading of a session that is not going to do anything else.
		return "idle", true
	default:
		return "", false
	}
}
