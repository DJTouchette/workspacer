package main

// `sessions.recent` — the resumable-session list, headless.
//
// TWIN: apps/desktop/src/main/services/recentSessions.ts (listRecentSessions /
// mergeRecentSessions).
//
// WHY IT MATTERED MORE THAN ITS SIZE SUGGESTS. Both shipped clients that call
// this SWALLOW its failure into an empty list — webBackend.ts returns [] and
// mobile.html catches and sets recents = []. So with no provider the Sessions
// pane and the phone's resume-a-recent-agent list did not report an error; they
// rendered as if the machine had no history at all. A wrong answer delivered
// confidently, which is worse than a visible failure and is exactly why this is
// worth porting even though the enrichment cannot be.
//
// WHAT IS AND IS NOT PORTED, precisely:
//
//   - The DAEMON half is the whole substance and it ports exactly. claudemon is
//     the truth for what is RESUMABLE — it never deletes a row, it stops and
//     archives them — so `GET /sessions?include_archived=true` is the list, and
//     mergeRecentSessions' filtering, defaulting and newest-first ordering come
//     with it.
//   - The desktop's SQLite session-history join (agent name, model, cost) has no
//     headless counterpart: that store is fed by the app's own hook accounting
//     and does not exist here. Those fields are reported EMPTY rather than
//     omitted, so a client reads "not known" from the same shape it always got.
//     The brain does have two naming sources of its own — spawn labels and the
//     persisted cwd→name renames enrich.go already reads — and uses them, so a
//     headless list is named wherever anything on this machine knows a name.
//   - Provider auto-titles are NOT ported. titleForSession reads each row's
//     transcript file off disk to pull claude's ai-title or first user message;
//     doing that per row on every call is a synchronous disk sweep this daemon
//     has no cache for, and an empty `title` is the same value the desktop
//     reports for a row whose transcript it could not read. Named here so the
//     absence is a decision rather than a discovery.

import (
	"context"
	"encoding/json"
	"sort"
	"strings"
	"time"
)

// recentSession is one row of the wire shape. TWIN: RecentAgentSession in
// shared/ipcTypes.ts, field for field — the renderer decodes this list without
// knowing which provider answered.
type recentSession struct {
	SessionID string  `json:"sessionId"`
	Provider  string  `json:"provider"`
	Cwd       string  `json:"cwd"`
	Mode      string  `json:"mode"`
	Transport string  `json:"transport"`
	Archived  bool    `json:"archived"`
	UpdatedAt int64   `json:"updatedAt"`
	StartedAt int64   `json:"startedAt"`
	Name      string  `json:"name"`
	Title     string  `json:"title"`
	Model     string  `json:"model"`
	CostUSD   float64 `json:"costUSD"`
}

// daemonSessionRow is claudemon's `GET /sessions` row, narrowed to what the
// list needs.
type daemonSessionRow struct {
	SessionID string `json:"session_id"`
	Cwd       string `json:"cwd"`
	Mode      string `json:"mode"`
	Provider  string `json:"provider"`
	Transport string `json:"transport"`
	UpdatedAt string `json:"updated_at"`
	StartedAt string `json:"started_at"`
	Archived  bool   `json:"archived"`
}

// mergeRecentSessions turns daemon rows into the wire list, newest first.
// TWIN: mergeRecentSessions, including both of its non-obvious rules:
//
//   - rows whose id starts with "agent-" are DROPPED. Those are the desktop's
//     synthetic pre-registration ids, not resumable daemon sessions.
//   - an empty `provider` means claude. Legacy daemon rows serialize it that
//     way, and reporting "" would leave the resume path with no binary to run.
func mergeRecentSessions(rows []daemonSessionRow, name func(row daemonSessionRow) string) []recentSession {
	out := make([]recentSession, 0, len(rows))
	for _, r := range rows {
		if r.SessionID == "" || strings.HasPrefix(r.SessionID, "agent-") {
			continue
		}
		provider := r.Provider
		if provider == "" {
			provider = "claude"
		}
		mode := r.Mode
		if mode == "" {
			mode = "unknown"
		}
		transport := r.Transport
		if transport == "" {
			transport = "pty"
		}
		label := ""
		if name != nil {
			label = name(r)
		}
		out = append(out, recentSession{
			SessionID: r.SessionID,
			Provider:  provider,
			Cwd:       r.Cwd,
			Mode:      mode,
			Transport: transport,
			Archived:  r.Archived,
			UpdatedAt: parseUnixMillis(r.UpdatedAt),
			StartedAt: parseUnixMillis(r.StartedAt),
			Name:      label,
			// The three the headless join cannot answer. Empty, not absent: the
			// client reads "not known" from the shape it already handles.
			Title:   "",
			Model:   "",
			CostUSD: 0,
		})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].UpdatedAt > out[j].UpdatedAt })
	return out
}

// parseUnixMillis mirrors the twin's `Date.parse(x) || 0`: an unparseable or
// missing timestamp sorts last rather than failing the list.
func parseUnixMillis(s string) int64 {
	if s == "" {
		return 0
	}
	for _, layout := range []string{time.RFC3339Nano, time.RFC3339} {
		if t, err := time.Parse(layout, s); err == nil {
			return t.UnixMilli()
		}
	}
	return 0
}

// recentSessions answers sessions.recent.
//
// A daemon that is down or mid-restart yields an EMPTY list rather than an
// error, matching the twin — the pane shows nothing instead of breaking the
// renderer. That is the one place this file reproduces the swallow it exists to
// fix, and it is the right call in only this direction: the failure is now the
// daemon being unreachable (which every other pane also shows), not the
// capability having no provider at all.
// It takes NO caller params, and the daemon's response is decoded into a local
// deliberately NOT named `raw`: in this package that name means "the caller's
// payload" and capspec_params_test.go's scan keys off it, so decoding a
// DAEMON response into a `raw` would report daemonSessionRow's cwd/mode as
// fields a caller can send — a decision demanded for params nobody has.
func (r *registry) recentSessions(ctx context.Context, _ json.RawMessage) (json.RawMessage, error) {
	body, err := r.cm.listSessionsIncludingArchived(ctx)
	if err != nil {
		return jsonResult([]recentSession{})
	}
	var rows []daemonSessionRow
	if json.Unmarshal(body, &rows) != nil {
		return jsonResult([]recentSession{})
	}
	byCwd := namesByCwd()
	return jsonResult(mergeRecentSessions(rows, func(row daemonSessionRow) string {
		// A spawn label wins over a cwd rename — the same precedence
		// enrichSnapshot applies to a live row, so a session does not change
		// name when it moves from the fleet list to the history list.
		if r.meta != nil {
			if m, ok := r.meta.get(row.SessionID); ok && m.Label != "" {
				return m.Label
			}
		}
		return byCwd[row.Cwd]
	}))
}
