package quiescence

import (
	"encoding/json"
	"fmt"
)

// ParseSessions reduces whatever a session provider answered to the fields the
// predicate reads. `peer` is empty for the local fleet, or the federated peer's
// name.
//
// It is deliberately tolerant about SHAPE and intolerant about MEANING. Two
// providers answer `sessions.snapshots` — the headless brain, which serves
// claudemon's rows (snake_case, carrying `mode`) with a desktop-shape overlay
// on top, and the Electron desktop, which serves its own camelCase snapshots
// and has no `mode` at all. Both are read. But a row this function cannot make
// sense of is marked [Session.Unreadable] rather than skipped, because a row
// nobody understands is not a row that is finished.
func ParseSessions(peer string, raw json.RawMessage) ([]Session, error) {
	if len(raw) == 0 {
		return nil, fmt.Errorf("empty answer")
	}
	var rows []map[string]any
	if err := json.Unmarshal(raw, &rows); err != nil {
		return nil, fmt.Errorf("unreadable session list: %w", err)
	}
	out := make([]Session, 0, len(rows))
	for i, row := range rows {
		out = append(out, parseSession(peer, i, row))
	}
	return out, nil
}

func parseSession(peer string, index int, row map[string]any) Session {
	s := Session{Peer: peer}
	s.ID = firstString(row, "sessionId", "session_id")
	if s.ID == "" {
		s.ID = fmt.Sprintf("row-%d", index)
		s.Unreadable = "no session id"
		return s
	}
	s.Mode = firstString(row, "mode")
	s.Ambient = firstString(row, "ambientState", "state")
	// `status: "ended"` is the desktop's word and `mode: "stopped"` is
	// claudemon's; either ends a session.
	s.Ended = firstString(row, "status") == "ended" || s.Mode == "stopped"
	s.BackgroundTasks = firstInt(row, "backgroundTasks", "background_tasks")

	// The pending slot arrives in two spellings: the desktop's split
	// pendingApproval / pendingQuestions fields, and claudemon's single
	// `pending` card. Read both — the compat overlay emits the first from the
	// second, but a raw row that skipped the overlay carries only the second.
	if v, ok := row["pendingApproval"]; ok && v != nil {
		s.PendingApproval = true
	}
	if q, ok := row["pendingQuestions"].([]any); ok && len(q) > 0 {
		s.PendingQuestion = true
	}
	if p, ok := row["pending"].(map[string]any); ok {
		switch p["kind"] {
		case "approval":
			s.PendingApproval = true
		case "question":
			s.PendingQuestion = true
		}
	}
	if s.Mode == "" && s.Ambient == "" && !s.Ended {
		// Neither state machine said anything. Not "idle": unreadable.
		s.Unreadable = "no mode and no ambientState on the row"
	}
	return s
}

func firstString(row map[string]any, keys ...string) string {
	for _, k := range keys {
		if v, ok := row[k].(string); ok && v != "" {
			return v
		}
	}
	return ""
}

func firstInt(row map[string]any, keys ...string) int {
	for _, k := range keys {
		switch v := row[k].(type) {
		case float64:
			if v > 0 {
				return int(v)
			}
		case int:
			if v > 0 {
				return v
			}
		}
	}
	return 0
}
