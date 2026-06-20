// Package event defines the bus envelope and topic matching. The same shape
// rides WebSocket frames today and will cross the MCP facade later, so it stays
// transport-agnostic and dependency-free.
package event

import (
	"encoding/json"
	"log"
	"strings"
	"time"
)

// Envelope is one event on the bus.
type Envelope struct {
	ID     string          `json:"id"`
	Type   string          `json:"type"`   // dotted topic, e.g. "agent.spawned"
	Source string          `json:"source"` // emitter: "hub", "claudemon", a plugin id
	Time   time.Time       `json:"time"`
	Data   json.RawMessage `json:"data,omitempty"`
}

// Matches reports whether an event type satisfies a subscription pattern.
//
//	"agent.spawned"  exact match
//	"agent.*"        any type under the "agent." namespace (not "agent" itself)
//	"*"              everything
func Matches(pattern, typ string) bool {
	if pattern == "*" || pattern == typ {
		return true
	}
	if strings.HasSuffix(pattern, ".*") {
		// TrimSuffix leaves the trailing dot, so "agent.*" -> prefix "agent."
		prefix := strings.TrimSuffix(pattern, "*")
		return strings.HasPrefix(typ, prefix)
	}
	return false
}

// MatchesAny reports whether typ matches at least one of the patterns.
func MatchesAny(patterns []string, typ string) bool {
	for _, p := range patterns {
		if Matches(p, typ) {
			return true
		}
	}
	return false
}

// New builds an envelope with data JSON-encoded. ID and Time are left blank for
// the broker to stamp at publish time.
func New(typ, source string, data any) Envelope {
	var raw json.RawMessage
	if data != nil {
		b, err := json.Marshal(data)
		if err != nil {
			log.Printf("event.New: failed to marshal data for type %q: %v", typ, err)
		} else {
			raw = b
		}
	}
	return Envelope{Type: typ, Source: source, Data: raw}
}
