package main

import (
	"encoding/json"
	"testing"
)

// THE ROUTING WIRE, END TO END INSIDE THE BRAIN: the three fields decode off
// agents.spawn, land in the session's spawn metadata, come back out on the
// snapshot, and are echoed in the spawn result.
//
// This is the "prove the value arrives" check for item 1. A recorded field that
// nothing reads is this fleet's most common bug and it looks exactly like a
// working one — the headless node is also the one MOST likely to be running a
// dispatched fleet and the one with no analytics store of its own, so a role it
// silently drops is a fleet whose analytics are wrong and whose decision log has
// no session to join to.
func TestTheRoutingFieldsDecodeRecordAndSurface(t *testing.T) {
	var p spawnParams
	raw := `{"cwd":"/w","role":"implementer","capability":"frontier","decisionId":"rd_deadbeef"}`
	if err := json.Unmarshal([]byte(raw), &p); err != nil {
		t.Fatalf("agents.spawn params did not decode: %v", err)
	}
	if p.Role != "implementer" || p.Capability != "frontier" || p.DecisionID != "rd_deadbeef" {
		t.Fatalf("the wire names do not match the struct tags: %+v", p)
	}
	if !p.routed() {
		t.Error("a spawn carrying all three fields does not count as routed, so none of it is recorded")
	}

	meta := newMetaStore()
	meta.set("s1", spawnMeta{Role: p.Role, Capability: p.Capability, DecisionID: p.DecisionID})

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/w"}`), meta)
	var m map[string]any
	if err := json.Unmarshal(out, &m); err != nil {
		t.Fatalf("enriched snapshot did not decode: %v", err)
	}
	routing, ok := m["routing"].(map[string]any)
	if !ok {
		t.Fatalf("the enriched snapshot carries no routing block, so nothing can join this session to the decision behind it: %s", out)
	}
	if routing["role"] != "implementer" || routing["capability"] != "frontier" || routing["decisionId"] != "rd_deadbeef" {
		t.Errorf("routing block = %v", routing)
	}

	// A spawn that carried none of them keeps the snapshot's exact shape.
	plain := enrichSnapshot(json.RawMessage(`{"session_id":"s2","cwd":"/w"}`), meta)
	var pm map[string]any
	_ = json.Unmarshal(plain, &pm)
	if _, present := pm["routing"]; present {
		t.Errorf("an unrouted session grew a routing block: %s", plain)
	}

	// …and the spawn ANSWER echoes what this host accepted, which is what tells
	// a dispatcher its capability was clamped by the hub on the way in.
	res := spawnResult("s1", "", false, p)
	echo, ok := res["routing"].(map[string]any)
	if !ok {
		t.Fatalf("the spawn result does not echo the routing fields: %v", res)
	}
	if echo["capability"] != "frontier" || echo["decisionId"] != "rd_deadbeef" {
		t.Errorf("spawn result routing echo = %v", echo)
	}
	if _, present := spawnResult("s2", "", false, spawnParams{})["routing"]; present {
		t.Errorf("an unrouted spawn's result grew a routing block")
	}
}
