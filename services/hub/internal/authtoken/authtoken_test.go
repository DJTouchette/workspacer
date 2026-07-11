package authtoken

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestParseScope(t *testing.T) {
	cases := []struct {
		in      string
		want    Scope
		wantErr bool
	}{
		{"view", ScopeView, false},
		{"triage", ScopeTriage, false},
		{"operator", ScopeOperator, false},
		{" Operator ", ScopeOperator, false}, // tolerate case/space from a CLI flag
		{"admin", "", true},
		{"", "", true},
	}
	for _, c := range cases {
		got, err := ParseScope(c.in)
		if (err != nil) != c.wantErr {
			t.Errorf("ParseScope(%q) err = %v, wantErr %v", c.in, err, c.wantErr)
			continue
		}
		if !c.wantErr && got != c.want {
			t.Errorf("ParseScope(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// TestScopeMethods pins the tier policy itself: which representative methods
// each tier admits, and that anything unlisted — including methods invented
// after this table — fails closed for view/triage.
func TestScopeMethods(t *testing.T) {
	allows := func(s Scope, method string) bool {
		for _, p := range s.Methods() {
			if p == "*" || p == method {
				return true
			}
			if strings.HasSuffix(p, ".*") && strings.HasPrefix(method, strings.TrimSuffix(p, "*")) {
				return true
			}
		}
		return false
	}
	cases := []struct {
		method                 string
		view, triage, operator bool
	}{
		// read-only surface
		{"agents.list", true, true, true},
		{"sessions.snapshots", true, true, true},
		{"sessions.transcript", true, true, true},
		{"layout.get", true, true, true},
		{"config.get", true, true, true},
		{"push.key", true, true, true},
		// acting on attention
		{"claude.approve", false, true, true},
		{"claude.answer", false, true, true},
		{"agents.sendMessage", false, true, true},
		{"claude.signal", false, true, true},
		{"push.subscribe", false, true, true},
		{"push.unsubscribe", false, true, true},
		// operator-only surface
		{"agents.spawn", false, false, true},
		{"terminals.create", false, false, true},
		{"sessions.terminalInput", false, false, true},
		{"git.push", false, false, true},
		{"git.commit", false, false, true},
		{"fs.write", false, false, true},
		{"config.save", false, false, true},
		{"layout.set", false, false, true},
		{"claude.setModel", false, false, true},
		// a method that doesn't exist yet must fail closed for scoped tiers
		{"future.unknownMethod", false, false, true},
	}
	for _, c := range cases {
		if got := allows(ScopeView, c.method); got != c.view {
			t.Errorf("view allows %q = %v, want %v", c.method, got, c.view)
		}
		if got := allows(ScopeTriage, c.method); got != c.triage {
			t.Errorf("triage allows %q = %v, want %v", c.method, got, c.triage)
		}
		if got := allows(ScopeOperator, c.method); got != c.operator {
			t.Errorf("operator allows %q = %v, want %v", c.method, got, c.operator)
		}
	}
	if Scope("bogus").Methods() != nil {
		t.Error("unknown scope must grant nothing (fail closed)")
	}
}

func TestMintPersistRoundtrip(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")

	rec, err := Mint(path, ScopeTriage, "dana's phone")
	if err != nil {
		t.Fatal(err)
	}
	if len(rec.Token) < 20 {
		t.Errorf("token suspiciously short: %q", rec.Token)
	}
	if rec.Scope != ScopeTriage || rec.Label != "dana's phone" || rec.Created.IsZero() {
		t.Errorf("record fields wrong: %+v", rec)
	}
	if runtime.GOOS != "windows" {
		st, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		if st.Mode().Perm() != 0o600 {
			t.Errorf("tokens.json mode = %v, want 0600 (bearer secrets)", st.Mode().Perm())
		}
	}

	// A second mint appends without clobbering the first.
	rec2, err := Mint(path, ScopeView, "")
	if err != nil {
		t.Fatal(err)
	}
	recs, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(recs) != 2 || recs[0].Token != rec.Token || recs[1].Token != rec2.Token {
		t.Fatalf("roundtrip = %+v, want both minted records in order", recs)
	}
}

func TestLoadMissingFileIsEmpty(t *testing.T) {
	recs, err := Load(filepath.Join(t.TempDir(), "nope.json"))
	if err != nil || recs != nil {
		t.Fatalf("Load(missing) = %v, %v; want nil, nil", recs, err)
	}
}

func TestRevoke(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	a, _ := Mint(path, ScopeView, "a")
	b, _ := Mint(path, ScopeTriage, "b")

	t.Run("by unique prefix", func(t *testing.T) {
		got, err := Revoke(path, a.Token[:12])
		if err != nil {
			t.Fatal(err)
		}
		if got.Token != a.Token {
			t.Errorf("revoked %q, want %q", got.Token, a.Token)
		}
		recs, _ := Load(path)
		if len(recs) != 1 || recs[0].Token != b.Token {
			t.Errorf("store after revoke = %+v, want only b", recs)
		}
	})
	t.Run("unknown ref errors", func(t *testing.T) {
		if _, err := Revoke(path, "zzzzzzzzzzzz"); err == nil {
			t.Error("revoking an unknown token should error, not silently no-op")
		}
	})
	t.Run("short ref refused", func(t *testing.T) {
		if _, err := Revoke(path, b.Token[:4]); err == nil {
			t.Error("a <8-char prefix must be refused (too easy to hit the wrong token)")
		}
	})
}

func TestRevokeAmbiguousPrefix(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	recs := []Record{
		{Token: "prefix-aaaaaaaa-1", Scope: ScopeView, Created: time.Now()},
		{Token: "prefix-aaaaaaaa-2", Scope: ScopeView, Created: time.Now()},
	}
	if err := Save(path, recs); err != nil {
		t.Fatal(err)
	}
	if _, err := Revoke(path, "prefix-aaaaaaaa"); err == nil {
		t.Error("ambiguous prefix must error rather than revoke an arbitrary match")
	}
}

// TestStoreLiveReload proves mint/revoke take effect on the running hub's next
// lookup: the Store re-reads tokens.json when it changes.
func TestStoreLiveReload(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	st := NewStore(path)

	if _, ok := st.Lookup("anything"); ok {
		t.Fatal("empty store resolved a token")
	}

	rec, err := Mint(path, ScopeTriage, "phone")
	if err != nil {
		t.Fatal(err)
	}
	got, ok := st.Lookup(rec.Token)
	if !ok || got.Scope != ScopeTriage {
		t.Fatalf("Lookup after mint = %+v, %v; want the triage record", got, ok)
	}

	if _, err := Revoke(path, rec.Token); err != nil {
		t.Fatal(err)
	}
	if _, ok := st.Lookup(rec.Token); ok {
		t.Error("Lookup after revoke still resolves — revocation must cut off new connections")
	}
}

func TestStoreCorruptFileFailsClosed(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := Mint(path, ScopeView, "")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	st := NewStore(path)
	if _, ok := st.Lookup(rec.Token); ok {
		t.Error("corrupt tokens.json must honor nothing, not stale grants")
	}
}
