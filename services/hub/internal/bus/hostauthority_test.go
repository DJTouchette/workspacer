package bus

import (
	"net/http"
	"testing"
)

// HOST AUTHORITY vs. "authorized".
//
// Authorized admits the host token AND any operator-tier scoped token, because
// an operator token is promoted to trusted at the handshake. That promotion is
// what makes a remote worker node work at all — a node attaches as a capability
// PROVIDER and providing requires trust — and it is also what put the node's
// bearer string one POST away from running code on the hub's host via the
// plugin-install family.
//
// HostAuthorized is the narrower question those routes ask: is this the host's
// OWN credential? These tests pin the difference in both directions, because
// each direction is a separate way to lose it — widening HostAuthorized back to
// Authorized re-opens the hole, and narrowing Authorized would break every
// remote client that legitimately holds an operator pairing.

// TestHostAuthorizedRefusesOperatorScopedTokens is the security claim: the one
// credential Authorized admits and HostAuthorized does not is the operator-tier
// scoped token — which is exactly what deploy/fly/node hands its brain.
func TestHostAuthorizedRefusesOperatorScopedTokens(t *testing.T) {
	_, srv := scopedServer(t)
	req := func(tok string) *http.Request {
		r, _ := http.NewRequest(http.MethodPost, "/plugins/install", nil)
		r.Header.Set("Authorization", "Bearer "+tok)
		return r
	}
	cases := []struct {
		token          string
		wantAuthorized bool
		wantHost       bool
		why            string
	}{
		{"host-secret", true, true, "the host's own pairing credential — the desktop, the CLI and `workspacer plugin dev` all present this"},
		{"tok-operator", true, false, "a node's credential: trusted on the bus, and still not the host"},
		{"tok-triage", false, false, "phone tier"},
		{"tok-view", false, false, "read-only tier"},
		{"bogus", false, false, "unknown string"},
		{"", false, false, "no credential at all"},
	}
	for _, c := range cases {
		if got := srv.Authorized(req(c.token)); got != c.wantAuthorized {
			t.Errorf("Authorized(%q) = %v, want %v (%s)", c.token, got, c.wantAuthorized, c.why)
		}
		if got := srv.HostAuthorized(req(c.token)); got != c.wantHost {
			t.Errorf("HostAuthorized(%q) = %v, want %v (%s)", c.token, got, c.wantHost, c.why)
		}
	}
	// The claim stated as the thing that would regress: if these two ever agree
	// on an operator token, the plugin-install family is back to accepting a
	// remote node's token.
	r := req("tok-operator")
	if srv.Authorized(r) == srv.HostAuthorized(r) {
		t.Fatal("HostAuthorized has collapsed into Authorized for the operator tier — a remote worker node's token can install plugins on this host again")
	}
}

// TestHostAuthorizedReadsTheQueryTokenToo: the credential can arrive as
// ?token= (a browser cannot set headers on a WebSocket handshake, and every
// hub client re-presents the same value both ways). A gate that only read the
// header would be bypassable by moving the token into the URL.
func TestHostAuthorizedReadsTheQueryTokenToo(t *testing.T) {
	_, srv := scopedServer(t)
	get := func(q string) *http.Request {
		r, _ := http.NewRequest(http.MethodPost, "/plugins/install?token="+q, nil)
		return r
	}
	if !srv.HostAuthorized(get("host-secret")) {
		t.Error("the host token in ?token= must pass — it is the same credential")
	}
	if srv.HostAuthorized(get("tok-operator")) {
		t.Fatal("an operator-scoped token in ?token= passed the host gate: the refusal is header-only and trivially bypassed")
	}
}

// TestHostAuthorizedIsOpenWhenNoTokenIsConfigured pins the loopback default.
// With no token there is no credential to hold, Authorized already answers true
// for everyone, and a host gate that refused would break every local dev run.
func TestHostAuthorizedIsOpenWhenNoTokenIsConfigured(t *testing.T) {
	srv := NewServer(nil)
	r, _ := http.NewRequest(http.MethodPost, "/plugins/install", nil)
	if !srv.HostAuthorized(r) {
		t.Fatal("with no token configured the hub is loopback-only and everything is the host; refusing here breaks local development")
	}
}

// TestScopedIdentForNamesTheRefusedCredential: the refusal logs which token it
// turned away (tier + label), so an operator can tell "my node is misbehaving"
// from "someone has my node's token". The host token is not a scoped ident and
// must not be reported as one.
func TestScopedIdentForNamesTheRefusedCredential(t *testing.T) {
	srv := NewServer(nil)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		if tok == "node-token" {
			return ScopedIdent{Scope: "operator", Methods: []string{"*"}, Label: "fly-node"}, true
		}
		return ScopedIdent{}, false
	})
	req := func(tok string) *http.Request {
		r, _ := http.NewRequest(http.MethodPost, "/plugins/install", nil)
		if tok != "" {
			r.Header.Set("Authorization", "Bearer "+tok)
		}
		return r
	}
	si, ok := srv.ScopedIdentFor(req("node-token"))
	if !ok {
		t.Fatal("a presented scoped token must resolve, or the refusal log cannot name it")
	}
	if si.Scope != "operator" || si.Label != "fly-node" {
		t.Fatalf("ScopedIdentFor = %+v, want the operator tier labelled fly-node", si)
	}
	if _, ok := srv.ScopedIdentFor(req("host-secret")); ok {
		t.Error("the host token is not a scoped ident and must not be described as one")
	}
	if _, ok := srv.ScopedIdentFor(req("")); ok {
		t.Error("no credential resolves to no ident")
	}
}
