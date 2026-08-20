package authtoken

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// The profile-dispatch grant (ProfilesAllowed) is recorded at mint time and
// read back by two independent consumers — the hub's handshake Store and the
// facade's per-request resolveRecord — through the same tokens.json. The
// round-trip is therefore the contract: the json tag, the persistence, and the
// mtime-gated Store all have to carry the grant unchanged.

func TestProfilesAllowedRoundTripsThroughSaveLoadAndStore(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	recs := []Record{
		{
			Token:           "manager-token",
			Scope:           ScopeOperator,
			Label:           "session:mgr-1",
			Created:         time.Now().UTC().Truncate(time.Second),
			Plugins:         []string{"djtouchette.jira"},
			ProfilesAllowed: []string{"work", "personal"},
		},
		{Token: "plain-token", Scope: ScopeView, Created: time.Now().UTC().Truncate(time.Second)},
	}
	if err := Save(path, recs); err != nil {
		t.Fatal(err)
	}

	loaded, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded) != 2 {
		t.Fatalf("loaded %d records, want 2", len(loaded))
	}
	if got := loaded[0].ProfilesAllowed; len(got) != 2 || got[0] != "work" || got[1] != "personal" {
		t.Fatalf("ProfilesAllowed did not round-trip: %v", got)
	}
	if loaded[1].ProfilesAllowed != nil {
		t.Fatalf("a grantless record grew a grant on the way through disk: %v", loaded[1].ProfilesAllowed)
	}

	st := NewStore(path)
	rec, ok := st.Lookup("manager-token")
	if !ok {
		t.Fatal("store lost the manager token")
	}
	if len(rec.ProfilesAllowed) != 2 || rec.ProfilesAllowed[0] != "work" {
		t.Fatalf("store dropped the profile grant: %v", rec.ProfilesAllowed)
	}
	if rec, _ := st.Lookup("plain-token"); rec.ProfilesAllowed != nil {
		t.Fatalf("store invented a grant: %v", rec.ProfilesAllowed)
	}
}

// TestProfilesAllowedWireShape pins the JSON spelling the desktop's TS mint
// path must write: camelCase `profilesAllowed`, omitted entirely when empty
// (consistent with its neighbor `plugins`). The desktop writes tokens.json
// directly, so the tag IS the cross-language contract.
func TestProfilesAllowedWireShape(t *testing.T) {
	b, err := json.Marshal(Record{Token: "x", Scope: ScopeOperator, ProfilesAllowed: []string{"work"}})
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatal(err)
	}
	if string(m["profilesAllowed"]) != `["work"]` {
		t.Fatalf("wire key/value drifted: %s", b)
	}
	b, err = json.Marshal(Record{Token: "x", Scope: ScopeView})
	if err != nil {
		t.Fatal(err)
	}
	if _, present := func() (json.RawMessage, bool) {
		var mm map[string]json.RawMessage
		_ = json.Unmarshal(b, &mm)
		v, ok := mm["profilesAllowed"]
		return v, ok
	}(); present {
		t.Fatalf("empty grant must be omitted (omitempty), got %s", b)
	}

	// And the read direction from a hand-shaped file, as the desktop writes it.
	path := filepath.Join(t.TempDir(), "tokens.json")
	raw := `[{"token":"m","scope":"operator","label":"session:s1","created":"2026-08-20T00:00:00Z","profilesAllowed":["work"]}]`
	if err := os.WriteFile(path, []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	recs, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(recs) != 1 || len(recs[0].ProfilesAllowed) != 1 || recs[0].ProfilesAllowed[0] != "work" {
		t.Fatalf("desktop-shaped record did not load: %+v", recs)
	}
}
