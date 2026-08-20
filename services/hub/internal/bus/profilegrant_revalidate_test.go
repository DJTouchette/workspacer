package bus

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

// A profile grant is snapshotted onto the connection at handshake (like the
// tier), so EDITING the grant — un-blessing a manager, or narrowing its
// accounts — must be treated exactly like a tier change: revalidateScoped
// closes the live socket, and the reconnect picks up the new grant. Without
// this, "revoke the manager's Work account" would be advisory against the one
// connection it was aimed at, which is the precise hole the scoped-revocation
// work already closed once for tiers.
func TestChangingAProfileGrantClosesTheLiveSocket(t *testing.T) {
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

	// The live socket must END within the revalidation window (the handshake
	// snapshot cannot be narrowed in place, so closing is the only honest act).
	// Detected by READ: a server-side close errors the read well inside the
	// budget; only the context deadline expiring means the socket stayed open.
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	for {
		if _, _, err := mgr.ws.Read(ctx); err != nil {
			if ctx.Err() != nil {
				t.Fatal("a manager whose profile grant was revoked kept its live socket (and its handshake-snapshot grant) past the revalidation window")
			}
			return // closed by the hub — the grant edit was enforced
		}
	}
}
