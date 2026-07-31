package bus

import (
	"encoding/json"
	"testing"
)

// A hub-owned capability that persists something on a caller's behalf — a Web
// Push registration is the case — outlives the connection that asked for it, so
// it has to record WHICH credential asked. RegisterLocalIdent is that seam: the
// handler sees the tier and a fingerprint of the presented token, never the
// token itself.
func TestLocalIdentHandlerSeesTheCallersCredential(t *testing.T) {
	url, srv := scopedServer(t)

	seen := make(chan CallerIdentity, 4)
	srv.RegisterLocalIdent("push.subscribe", func(c CallerIdentity, _ json.RawMessage) (any, error) {
		seen <- c
		return map[string]any{"ok": true}, nil
	})

	triage := dialClientToken(t, url, "tok-triage")
	triage.send(Frame{Op: "call", ID: "c1", Method: "push.subscribe"})
	triage.readUntil("result")
	got := <-seen
	if got.Scope != "triage" || got.Trusted {
		t.Fatalf("identity = %+v, want the triage tier, untrusted", got)
	}
	if got.TokenID != TokenFingerprint("tok-triage") {
		t.Fatalf("TokenID = %q, want the fingerprint of the presented token", got.TokenID)
	}
	if got.TokenID == "tok-triage" {
		t.Fatal("the raw token reached the handler — only a fingerprint may")
	}

	// The host token is the same authority as an operator token and reports it.
	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "call", ID: "c2", Method: "push.subscribe"})
	host.readUntil("result")
	if got := <-seen; !got.Trusted || got.Scope != "operator" {
		t.Fatalf("host identity = %+v, want trusted/operator", got)
	}

	// Two devices on the same token share an identity — that is what makes
	// revoking the token cut all of them at once.
	second := dialClientToken(t, url, "tok-triage")
	second.send(Frame{Op: "call", ID: "c3", Method: "push.subscribe"})
	second.readUntil("result")
	if again := <-seen; again.TokenID != TokenFingerprint("tok-triage") {
		t.Fatalf("second connection on the same token got %q, want the same fingerprint", again.TokenID)
	}
}

// Fingerprints must be stable (a restart can't invalidate what was stored),
// distinct per token, and empty for "no credential presented" — the loopback
// default, where there is no identity to record.
func TestTokenFingerprint(t *testing.T) {
	if TokenFingerprint("") != "" {
		t.Fatal("no token must fingerprint to the empty identity, not to a hash of nothing")
	}
	a, b := TokenFingerprint("tok-a"), TokenFingerprint("tok-b")
	if a == b {
		t.Fatal("distinct tokens collided")
	}
	if a != TokenFingerprint("tok-a") {
		t.Fatal("fingerprint is not stable across calls")
	}
	if a == "tok-a" {
		t.Fatal("fingerprint returned the token itself")
	}
}

// A method registered as identity-aware must not still be reachable through a
// stale plain registration (and vice versa), or which handler answers would
// depend on registration order.
func TestLocalRegistrationsReplaceEachOther(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.RegisterLocal("layout.get", func(json.RawMessage) (any, error) {
		return "plain", nil
	})
	srv.RegisterLocalIdent("layout.get", func(CallerIdentity, json.RawMessage) (any, error) {
		return "ident", nil
	})
	c := dialClient(t, url)
	c.send(Frame{Op: "call", ID: "r1", Method: "layout.get"})
	if got := string(c.readUntil("result").Result); got != `"ident"` {
		t.Fatalf("result = %s, want the identity-aware handler to have replaced the plain one", got)
	}

	srv.RegisterLocal("layout.get", func(json.RawMessage) (any, error) {
		return "plain", nil
	})
	c.send(Frame{Op: "call", ID: "r2", Method: "layout.get"})
	if got := string(c.readUntil("result").Result); got != `"plain"` {
		t.Fatalf("result = %s, want the plain handler to have replaced the identity-aware one", got)
	}
	if n := srv.router.methodCount(); n != 1 {
		t.Fatalf("methodCount = %d, want the method counted once", n)
	}
}
