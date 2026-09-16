package bus

import (
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

// Legacy profile fields remain parse-compatible but are inert. Editing one
// must not disconnect a live token or narrow profile selection.
func TestChangingALegacyProfileFieldDoesNotCloseTheLiveSocket(t *testing.T) {
	restore := shortenScopedRevalidation(t)
	defer restore()

	file := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := authtoken.Mint(file, authtoken.ScopeOperator, "manager")
	if err != nil {
		t.Fatal(err)
	}
	// Bless the manager for "work" the way the desktop does: rewrite the record.
	recs, err := authtoken.Load(file)
	if err != nil {
		t.Fatal(err)
	}
	recs[0].ProfilesAllowed = []string{"work"}
	if err := authtoken.Save(file, recs); err != nil {
		t.Fatal(err)
	}
	store := authtoken.NewStore(file)

	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		r, ok := store.Lookup(tok)
		if !ok {
			return ScopedIdent{}, false
		}
		return ScopedIdent{Scope: string(r.Scope), Methods: r.Scope.Methods(),
			ProfilesAllowed: r.ProfilesAllowed}, true
	})

	// A provider so the manager's spawns actually round-trip.
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"agents.spawn"}})
	provider.readUntil("registered")
	go func() {
		for {
			f, ok := provider.tryRead("call")
			if !ok {
				return
			}
			provider.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
		}
	}()

	mgr := dialClientToken(t, url, rec.Token)

	// FLOOR: while blessed, the manager's granted spawn round-trips.
	mgr.send(Frame{Op: "call", ID: "f1", Method: "agents.spawn",
		Params: json.RawMessage(`{"profileId":"work"}`)})
	if _, ok := mgr.tryReadUntil("result", "result", 2*time.Second); !ok {
		t.Fatal("floor: a blessed manager's spawn must round-trip")
	}

	// Un-bless: same token, same tier, grant emptied.
	recs, err = authtoken.Load(file)
	if err != nil {
		t.Fatal(err)
	}
	recs[0].ProfilesAllowed = nil
	if err := authtoken.Save(file, recs); err != nil {
		t.Fatal(err)
	}

	time.Sleep(250 * time.Millisecond)
	mgr.send(Frame{Op: "call", ID: "f2", Method: "agents.spawn",
		Params: json.RawMessage(`{"profileId":"work"}`)})
	if _, ok := mgr.tryReadUntil("result", "result", 2*time.Second); !ok {
		t.Fatal("legacy profile field edit disconnected or narrowed the live token")
	}
}
