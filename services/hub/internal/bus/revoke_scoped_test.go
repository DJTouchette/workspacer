package bus

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// PROVEN, critical. Revoking a scoped user token did not close its live bus
// socket: the revoked phone kept receiving the whole event firehose and kept
// making capability calls, for the life of the hub process.
//
// The two planes disagreed about the same act. `workspacer token revoke`
// rewrites tokens.json; Store.Lookup re-reads it, so a NEW dial is correctly
// 401'd — the hub's own comment says revoking "takes effect on the next
// connection". Nothing re-checked a connection that was ALREADY open: handshake
// classification runs once, conn.scopeMethods is a snapshot, and mayCall /
// mayConsume answered from that snapshot forever.
//
// The same hole was found and fixed for PLUGIN tokens a round earlier
// (TestRevocationClosesTheConnectionAndStopsEventDelivery, whose comment reads
// "an uninstalled plugin's sidecar went on receiving every event type its
// manifest declared, for the life of the hub process, on a socket the host
// believed it had revoked"). The phone/web-remote tier — the one credential a
// user is actually expected to revoke, on the one device that gets lost — never
// got the same treatment.
//
// This test uses the REAL on-disk store: authtoken.Mint writes tokens.json,
// SetScopedTokenLookup is wired exactly as cmd/hub/main.go wires it, and
// authtoken.Revoke rewrites the same file.
func TestRevokingAScopedTokenClosesItsLiveSocket(t *testing.T) {
	restore := shortenScopedRevalidation(t)
	defer restore()

	file := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := authtoken.Mint(file, authtoken.ScopeView, "phone")
	if err != nil {
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
		return ScopedIdent{Scope: string(r.Scope), Methods: r.Scope.Methods()}, true
	})

	phone := dialClientToken(t, url, rec.Token)
	phone.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	phone.readUntil("subscribed")

	// FLOOR: while the token is live, this socket really does receive the feed.
	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.snapshot",
		Data: json.RawMessage(`{"sessionId":"REAL-1","cwd":"/home/u/secret-project","tag":"before-revoke"}`)}})
	if got, ok := phone.tryReadUntil("event", "event", 2*time.Second); !ok || got.Event == nil {
		t.Fatal("floor: a live view token subscribed to \"*\" must receive agent.snapshot")
	}

	if _, err := authtoken.Revoke(file, rec.Token); err != nil {
		t.Fatal(err)
	}

	// The socket must END. A read on a closed connection fails fast; a read on
	// an open one blocks to the deadline — the two are told apart by TIME.
	start := time.Now()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.snapshot",
			Data: json.RawMessage(`{"sessionId":"REAL-1","cwd":"/home/u/secret-project","tag":"after-revoke"}`)}})
		got, ok := phone.tryReadUntil("event", "event", 200*time.Millisecond)
		if !ok {
			// Connection closed (or nothing more arrives) — check it is really
			// closed by attempting a call.
			phone.send(Frame{Op: "call", ID: "c1", Method: "agents.list"})
			if r, ok := phone.tryReadUntil("result", "result", 500*time.Millisecond); ok {
				t.Fatalf("a REVOKED view token still got a result for agents.list: %s", r.Result)
			}
			phone.ws.CloseNow()
			return
		}
		if strings.Contains(string(got.Event.Data), "after-revoke") {
			if time.Since(start) > 2*time.Second {
				break
			}
			continue
		}
	}
	t.Fatal("a revoked scoped token's socket was still delivering events after revocation — revocation is advisory against an established connection, which is the only situation it exists for")
}

// The operator tier is the worse half: an operator-scoped token is promoted to
// `trusted` at handshake, so a revoked operator socket kept FULL host authority
// — every guarded topic (mayConsume's trusted short-circuit) and the right to
// publish anything.
func TestRevokingAnOperatorTokenAlsoClosesItsSocket(t *testing.T) {
	restore := shortenScopedRevalidation(t)
	defer restore()

	file := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := authtoken.Mint(file, authtoken.ScopeOperator, "laptop")
	if err != nil {
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
		return ScopedIdent{Scope: string(r.Scope), Methods: r.Scope.Methods()}, true
	})

	op := dialClientToken(t, url, rec.Token)
	if op.hello.Scope != "operator" {
		t.Fatalf("hello scope = %q, want operator", op.hello.Scope)
	}
	op.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	op.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.s1"}})
	if got, ok := op.tryReadUntil("event", "event", 2*time.Second); !ok || got.Event == nil {
		t.Fatal("floor: a live operator token must receive the guarded stream it holds the capability for")
	}

	if _, err := authtoken.Revoke(file, rec.Token); err != nil {
		t.Fatal(err)
	}

	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.s1",
			Data: json.RawMessage(`"YWZ0ZXItcmV2b2tl"`)}})
		got, ok := op.tryReadUntil("event", "event", 200*time.Millisecond)
		if !ok {
			op.ws.CloseNow()
			return
		}
		if strings.Contains(string(got.Event.Data), "YWZ0ZXItcmV2b2tl") && time.Since(deadline) > 0 {
			break
		}
	}
	t.Fatal("a revoked OPERATOR token's socket was still delivering guarded topics — it is promoted to trusted at handshake, so the snapshot it kept is full host authority")
}

// A tier DOWNGRADE is a revocation of the difference. The record still resolves,
// so a plain "does this token exist" re-check would miss it entirely.
func TestDowngradingAScopedTokensTierClosesItsSocket(t *testing.T) {
	restore := shortenScopedRevalidation(t)
	defer restore()

	// Read by the server's revalidation goroutine while this test writes it.
	var tier atomic.Value
	tier.Store(authtoken.ScopeOperator)
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		if tok != "tok-mutable" {
			return ScopedIdent{}, false
		}
		cur := tier.Load().(authtoken.Scope)
		return ScopedIdent{Scope: string(cur), Methods: cur.Methods()}, true
	})

	c := dialClientToken(t, url, "tok-mutable")
	c.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	c.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.s1"}})
	if _, ok := c.tryReadUntil("event", "event", 2*time.Second); !ok {
		t.Fatal("floor: an operator socket must receive pty.bytes")
	}

	tier.Store(authtoken.ScopeView)

	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.s1"}})
		if _, ok := c.tryReadUntil("event", "event", 200*time.Millisecond); !ok {
			c.ws.CloseNow()
			return
		}
	}
	t.Fatal("a token downgraded from operator to view kept its operator socket, so the demotion applied to nothing that was already connected")
}

// shortenScopedRevalidation makes the poll interval test-sized. The production
// value is seconds; a test that waited for it would be a five-second test, and a
// test that mocked the loop away would not be testing the loop.
func shortenScopedRevalidation(t *testing.T) func() {
	t.Helper()
	prev := scopedRevalidateNanos.Load()
	scopedRevalidateNanos.Store(int64(20 * time.Millisecond))
	return func() { scopedRevalidateNanos.Store(prev) }
}
