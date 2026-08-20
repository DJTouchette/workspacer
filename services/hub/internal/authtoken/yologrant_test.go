package authtoken

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// TestYoloAllowedWireShape pins the JSON spelling the desktop's TS mint path
// must write for the full-access grant: camelCase `yoloAllowed`, omitted
// entirely when false (consistent with its neighbor `profilesAllowed`). The
// desktop writes tokens.json directly, so the tag IS the cross-language
// contract — twin of TestProfilesAllowedWireShape.
func TestYoloAllowedWireShape(t *testing.T) {
	b, err := json.Marshal(Record{Token: "x", Scope: ScopeOperator, YoloAllowed: true})
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatal(err)
	}
	if string(m["yoloAllowed"]) != `true` {
		t.Fatalf("wire key/value drifted: %s", b)
	}
	b, err = json.Marshal(Record{Token: "x", Scope: ScopeOperator})
	if err != nil {
		t.Fatal(err)
	}
	var mm map[string]json.RawMessage
	if err := json.Unmarshal(b, &mm); err != nil {
		t.Fatal(err)
	}
	if _, present := mm["yoloAllowed"]; present {
		t.Fatalf("an ungranted record must omit the key (omitempty), got %s", b)
	}

	// And the read direction from a hand-shaped file, as the desktop writes it —
	// through Save/Load AND the mtime-gated Store, both consumers of the grant.
	path := filepath.Join(t.TempDir(), "tokens.json")
	raw := `[{"token":"m","scope":"operator","label":"session:s1","created":"2026-08-20T00:00:00Z","yoloAllowed":true},` +
		`{"token":"p","scope":"view","created":"2026-08-20T00:00:00Z"}]`
	if err := os.WriteFile(path, []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	recs, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(recs) != 2 || !recs[0].YoloAllowed {
		t.Fatalf("desktop-shaped record did not load: %+v", recs)
	}
	if recs[1].YoloAllowed {
		t.Fatalf("a grantless record grew the full-access grant on the way through disk: %+v", recs[1])
	}
	if err := Save(path, recs); err != nil {
		t.Fatal(err)
	}
	st := NewStore(path)
	rec, ok := st.Lookup("m")
	if !ok || !rec.YoloAllowed {
		t.Fatalf("store dropped the full-access grant: %+v", rec)
	}
	if rec, _ := st.Lookup("p"); rec.YoloAllowed {
		t.Fatalf("store invented a grant: %+v", rec)
	}
}
