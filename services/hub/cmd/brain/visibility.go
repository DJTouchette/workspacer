package main

// Shared fleet-visibility rule — "what the Electron desktop sidebar shows" —
// applied to the brain's agents.list / sessions.snapshots results and its
// agent.snapshot publishes, so every bus client (/m, /app, plugins, MCP) sees
// the same fleet as the desktop instead of claudemon's full resumable history.
//
// The desktop rule this mirrors:
//   - LIVE sessions (claudemon mode != "stopped"), idle included, are always
//     shown: the app's claudeSessionStore holds every non-ended session (an
//     ended one is evicted ~30s after SessionEnd, claudeSessionStore.ts) and
//     the sidebar auto-adopts new live sessions (App.tsx).
//   - STOPPED sessions are shown only when curated — referenced by the shared
//     layout document's agent cards (sessionId / lastSessionId / a pane's
//     attachSessionId), which the sidebar renders as "Stopped — respawn"
//     (useAgentManager.reconcileAgents). The desktop never lists claudemon's
//     other stopped-but-resumable rows; those are reachable only via explicit
//     resume (App.tsx preexistingSessionIdsRef).
//   - Fallback (documented choice): when NO layout document exists — a pure
//     headless `workspacer serve` that never met a desktop — a stopped,
//     non-archived session stays visible for 24h after its last update, so a
//     phone isn't blind to an agent that just finished. When a layout exists,
//     the layout alone decides, exactly like the desktop.

import (
	"context"
	"encoding/json"
	"sync"
	"time"
)

// stoppedVisibleWindow is how long a stopped session stays visible when no
// layout document exists to curate the fleet (see the fallback note above).
const stoppedVisibleWindow = 24 * time.Hour

// layoutFetcher returns the raw hub-local layout.get result ({version, data}).
type layoutFetcher func(ctx context.Context) (json.RawMessage, error)

// visibility evaluates the rule against the hub's shared layout document,
// cached briefly so per-snapshot checks (every agent.snapshot publish) don't
// hammer layout.get.
type visibility struct {
	fetch layoutFetcher
	ttl   time.Duration
	now   func() time.Time

	mu        sync.Mutex
	ids       map[string]bool // curated session ids from the layout
	hasLayout bool
	fetchedAt time.Time // last fetch attempt (success or not) — throttles retries
	primed    bool      // at least one successful fetch landed
}

func newVisibility(fetch layoutFetcher, ttl time.Duration) *visibility {
	return &visibility{fetch: fetch, ttl: ttl, now: time.Now}
}

// layoutState returns the curated session-id set and whether a layout document
// exists, refreshing the cache when stale. A failed fetch keeps the last known
// state (or "no layout" before the first success) and still bumps fetchedAt so
// a dead hub isn't re-dialed on every snapshot.
func (v *visibility) layoutState(ctx context.Context) (map[string]bool, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	if !v.fetchedAt.IsZero() && v.now().Sub(v.fetchedAt) < v.ttl {
		return v.ids, v.hasLayout
	}
	v.fetchedAt = v.now()
	fctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	if raw, err := v.fetch(fctx); err == nil {
		v.ids, v.hasLayout = layoutSessionIDs(raw)
		v.primed = true
	}
	return v.ids, v.hasLayout
}

// visible applies the rule to one snapshot.
func (v *visibility) visible(ctx context.Context, snap json.RawMessage) bool {
	ids, hasLayout := v.layoutState(ctx)
	return snapshotVisible(snap, ids, hasLayout, v.now())
}

// filter returns only the snapshots the rule shows.
func (v *visibility) filter(ctx context.Context, snaps []json.RawMessage) []json.RawMessage {
	ids, hasLayout := v.layoutState(ctx)
	now := v.now()
	out := make([]json.RawMessage, 0, len(snaps))
	for _, s := range snaps {
		if snapshotVisible(s, ids, hasLayout, now) {
			out = append(out, s)
		}
	}
	return out
}

// snapshotVisible applies the shared rule to one claudemon-shaped session
// snapshot (post-enrichment, so it also honors a desktop-shaped status field).
func snapshotVisible(snap json.RawMessage, layoutIDs map[string]bool, hasLayout bool, now time.Time) bool {
	var s struct {
		SessionID string `json:"session_id"`
		Mode      string `json:"mode"`
		Status    string `json:"status"`
		Archived  bool   `json:"archived"`
		UpdatedAt string `json:"updated_at"`
	}
	if json.Unmarshal(snap, &s) != nil {
		return false
	}
	// mode "unknown" = no hook or managed-mode signal yet: shell terminals
	// (which never emit one) and agents still in TUI startup/OAuth. The
	// desktop's claudeSessionStore only ever contains sessions that produced a
	// signal, so its bus surface excludes exactly these rows — mirror that.
	if s.Mode == "unknown" {
		return false
	}
	stopped := s.Mode == "stopped" || (s.Mode == "" && s.Status == "ended")
	if !stopped {
		return true // live — idle included — is always visible
	}
	if layoutIDs[s.SessionID] {
		return true // curated: the desktop sidebar shows it as a stopped card
	}
	if hasLayout {
		return false // a layout exists and doesn't reference it — hidden
	}
	// No layout (pure headless): recently-stopped fallback.
	if s.Archived {
		return false
	}
	t, err := time.Parse(time.RFC3339, s.UpdatedAt)
	if err != nil {
		return false
	}
	return now.Sub(t) <= stoppedVisibleWindow
}

// layoutSessionIDs extracts every session id the shared layout curates: each
// non-global agent card's sessionId / lastSessionId plus its panes'
// attachSessionId. hasLayout is false when the document is null/absent (a hub
// that never held a layout), which switches the rule to the recency fallback.
func layoutSessionIDs(result json.RawMessage) (ids map[string]bool, hasLayout bool) {
	var doc struct {
		Data struct {
			Agents []struct {
				Global        bool   `json:"global"`
				SessionID     string `json:"sessionId"`
				LastSessionID string `json:"lastSessionId"`
				Tabs          []struct {
					Panes []struct {
						AttachSessionID string `json:"attachSessionId"`
					} `json:"panes"`
				} `json:"tabs"`
			} `json:"agents"`
		} `json:"data"`
	}
	if json.Unmarshal(result, &doc) != nil || doc.Data.Agents == nil {
		return nil, false
	}
	ids = map[string]bool{}
	for _, a := range doc.Data.Agents {
		if a.Global {
			continue // the Overview workspace is not an agent card
		}
		if a.SessionID != "" {
			ids[a.SessionID] = true
		}
		if a.LastSessionID != "" {
			ids[a.LastSessionID] = true
		}
		for _, t := range a.Tabs {
			for _, p := range t.Panes {
				if p.AttachSessionID != "" {
					ids[p.AttachSessionID] = true
				}
			}
		}
	}
	return ids, true
}
