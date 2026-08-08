package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

// alwaysVisible is a fleet rule that shows everything it is handed. It is the
// instrument, not the subject: it removes the visibility filter from the
// equation so what remains is exactly what visibleStatusLinePublisher refuses on
// its own.
type alwaysVisible struct{ asked []string }

func (a *alwaysVisible) visible(_ context.Context, snap json.RawMessage) bool {
	a.asked = append(a.asked, string(snap))
	return true
}

// THE UNKNOWN-SESSION ARM, which the visibility test could not see.
//
// TestStatusLinePublishHonoursFleetVisibility already calls
// pub("UNKNOWN-TO-THE-STORE", …) and asserts nothing is published — and
// `if !ok { return }` could still be deleted with that test green, because
// store.get returns a nil snapshot for a miss and snapshotVisible's
// json.Unmarshal errors on nil, so the OTHER guard swallowed the case. The arm
// was covered by an accident in a different function.
//
// It is one byte from not holding: `null` (as opposed to nil) unmarshals
// cleanly into the zero struct, mode "" and status not "ended", which
// snapshotVisible reads as LIVE. So the store question is asked here on its own,
// against a rule that says yes to everything.
func TestStatusLinePublishRefusesASessionTheStoreDoesNotHave(t *testing.T) {
	store := newSessionStore()
	store.seed(map[string]json.RawMessage{
		"SHOWN-1": json.RawMessage(`{"session_id":"SHOWN-1","mode":"running"}`),
	})
	vis := &alwaysVisible{}

	var published []string
	pub := visibleStatusLinePublisher(store, vis, func(topic string, payload json.RawMessage) {
		published = append(published, topic+" "+string(payload))
	})

	sl := json.RawMessage(`{"model_display":"opus","cost_usd":41.72,"five_hour_pct":93.0}`)
	pub("UNKNOWN-TO-THE-STORE", sl)
	pub("SHOWN-1", sl)

	for _, got := range published {
		if strings.Contains(got, "UNKNOWN-TO-THE-STORE") {
			t.Errorf("agent.statusline published for a session the store has no snapshot for: %s\nIt announces the id, model, cost and rate-limit state of a session the store does not admit to having — and sessions.snapshot(id) is view-callable and unfiltered by id, so the id completes the read.", got)
		}
	}
	// The rule must never have been consulted about the unknown id: a nil
	// snapshot reaching it is the accident this test exists to stop depending on.
	for _, asked := range vis.asked {
		if asked == "" || asked == "null" {
			t.Errorf("the fleet rule was asked about an empty snapshot (%q). Whether that answers false is snapshotVisible's business, and `null` decodes to a zero struct it calls LIVE.", asked)
		}
	}
	// FLOOR: a session the store DOES have, under a rule that shows it, must
	// publish — or the guard is a mute button.
	if len(published) != 1 || !strings.Contains(published[0], "SHOWN-1") || !strings.Contains(published[0], "cost_usd") {
		t.Fatalf("the known session's statusline did not publish; got %v", published)
	}
}

// A publisher with no fleet rule installed publishes NOTHING. The whole point of
// this function is that agent.statusline was the one fleet publish with no
// visibility filter; "no filter" must not resolve back to "publish everything",
// and a nil rule must not panic on a goroutine that would take the hub with it.
func TestStatusLinePublishWithNoVisibilityRulePublishesNothing(t *testing.T) {
	store := newSessionStore()
	store.seed(map[string]json.RawMessage{
		"HIDDEN-1": json.RawMessage(`{"session_id":"HIDDEN-1","mode":"stopped","updated_at":"2020-01-01T00:00:00Z"}`),
		"SHOWN-1":  json.RawMessage(`{"session_id":"SHOWN-1","mode":"running"}`),
	})
	var published []string
	pub := visibleStatusLinePublisher(store, nil, func(topic string, payload json.RawMessage) {
		published = append(published, topic+" "+string(payload))
	})
	pub("SHOWN-1", json.RawMessage(`{"cost_usd":41.72}`))
	pub("HIDDEN-1", json.RawMessage(`{"cost_usd":41.72}`))
	if len(published) != 0 {
		t.Fatalf("a publisher with no fleet rule published %v — unfiltered is the state this function exists to end", published)
	}
}
