package authtoken

import (
	"encoding/json"
	"path/filepath"
	"testing"
)

// TestRoleWireShapeAndRoundTrip pins the JSON spelling of the desktop-owned
// session-role tag (`role`, omitted when empty — TWIN: RemoteTokenRecord.role
// in ipcTypes.ts) and, the part Go is actually responsible for, that the CLI's
// Load→Save rewrites PRESERVE it: before the field existed, a `workspacer
// token create` re-marshal silently stripped every session token's role, and
// with it the desktop's ability to reconcile full-access grants on a config
// flip.
func TestRoleWireShapeAndRoundTrip(t *testing.T) {
	b, err := json.Marshal(Record{Token: "x", Scope: ScopeOperator, Role: "manager"})
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatal(err)
	}
	if string(m["role"]) != `"manager"` {
		t.Fatalf("role must marshal as camelCase `role`, got %s", b)
	}
	bb, err := json.Marshal(Record{Token: "x", Scope: ScopeOperator})
	if err != nil {
		t.Fatal(err)
	}
	var mm map[string]json.RawMessage
	if err := json.Unmarshal(bb, &mm); err != nil {
		t.Fatal(err)
	}
	if _, present := mm["role"]; present {
		t.Fatalf("empty role must be omitted, got %s", bb)
	}

	// Load→Save round trip (what Mint/Revoke do to the whole file).
	path := filepath.Join(t.TempDir(), "tokens.json")
	if err := Save(path, []Record{{Token: "m", Scope: ScopeOperator, Label: "session:s1", Role: "manager", YoloAllowed: true}}); err != nil {
		t.Fatal(err)
	}
	if _, err := Mint(path, ScopeView, "pairing"); err != nil {
		t.Fatal(err)
	}
	recs, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(recs) != 2 || recs[0].Role != "manager" || !recs[0].YoloAllowed {
		t.Fatalf("a rewrite of tokens.json must preserve role + grants: %+v", recs)
	}
}
