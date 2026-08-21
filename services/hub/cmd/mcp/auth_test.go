package main

import (
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
)

func TestCheckBindPolicy(t *testing.T) {
	cases := []struct {
		name      string
		addr      string
		token     string
		untokened string
		wantErr   bool
	}{
		{"loopback ip, no token, allowed", "127.0.0.1:7897", "", untokenedOperator, false},
		{"loopback name, no token, allowed", "localhost:7897", "", untokenedOperator, false},
		{"ipv6 loopback, no token, allowed", "[::1]:7897", "", untokenedOperator, false},
		{"bare port, no token, refused", ":7897", "", untokenedOperator, true},
		{"all interfaces, no token, refused", "0.0.0.0:7897", "", untokenedOperator, true},
		{"lan ip, no token, refused", "192.168.1.10:7897", "", untokenedOperator, true},
		{"lan ip, with token, allowed", "192.168.1.10:7897", "s3cret", untokenedOperator, false},
		{"all interfaces, with token, allowed", "0.0.0.0:7897", "s3cret", untokenedOperator, false},
		// -untokened deny refuses every credential-less request, which is
		// strictly stronger than requiring the static token — it satisfies the
		// non-loopback bind policy on its own.
		{"lan ip, no token, untokened deny, allowed", "192.168.1.10:7897", "", untokenedDeny, false},
		{"bare port, no token, untokened deny, allowed", ":7897", "", untokenedDeny, false},
		// view still serves tools to anyone who reaches the port — not enough.
		{"lan ip, no token, untokened view, refused", "192.168.1.10:7897", "", untokenedView, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			err := checkBindPolicy(c.addr, c.token, c.untokened)
			if (err != nil) != c.wantErr {
				t.Fatalf("checkBindPolicy(%q, tokenSet=%v, untokened=%q) err = %v, wantErr = %v", c.addr, c.token != "", c.untokened, err, c.wantErr)
			}
		})
	}
}

func TestIsLoopbackAddr(t *testing.T) {
	cases := []struct {
		addr string
		want bool
	}{
		{"127.0.0.1:7897", true},
		{"localhost:7897", true},
		{"[::1]:7897", true},
		{"127.0.0.1", true}, // no port
		{":7897", false},
		{"0.0.0.0:7897", false},
		{"::", false},
		{"192.168.1.10:7897", false},
		{"example.com:7897", false}, // unresolved hostname → fail safe
	}
	for _, c := range cases {
		if got := isLoopbackAddr(c.addr); got != c.want {
			t.Errorf("isLoopbackAddr(%q) = %v, want %v", c.addr, got, c.want)
		}
	}
}

// mintTestToken writes a tokens.json with one scoped record and returns the
// store plus the token value.
func mintTestToken(t *testing.T, scope authtoken.Scope) (*authtoken.Store, string) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := authtoken.Mint(path, scope, "session:test")
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	return authtoken.NewStore(path), rec.Token
}

func resolveReq(gate *authGate, target string, header string) (authtoken.Scope, bool) {
	req := httptest.NewRequest(http.MethodGet, target, nil)
	if header != "" {
		req.Header.Set("Authorization", header)
	}
	return gate.resolve(req)
}

func TestAuthGateResolve(t *testing.T) {
	store, viewTok := mintTestToken(t, authtoken.ScopeView)

	t.Run("no credential, no static token → operator (loopback-open default)", func(t *testing.T) {
		gate := &authGate{store: store}
		scope, ok := resolveReq(gate, "/mcp", "")
		if !ok || scope != authtoken.ScopeOperator {
			t.Fatalf("resolve = (%q, %v), want (operator, true)", scope, ok)
		}
	})

	t.Run("no credential with static token → refused", func(t *testing.T) {
		gate := &authGate{static: "s3cret", store: store}
		if _, ok := resolveReq(gate, "/mcp", ""); ok {
			t.Fatal("expected refusal without a credential when a static token is set")
		}
	})

	t.Run("static token bearer → operator", func(t *testing.T) {
		gate := &authGate{static: "s3cret", store: store}
		scope, ok := resolveReq(gate, "/mcp", "Bearer s3cret")
		if !ok || scope != authtoken.ScopeOperator {
			t.Fatalf("resolve = (%q, %v), want (operator, true)", scope, ok)
		}
	})

	t.Run("scoped token bearer → its tier", func(t *testing.T) {
		gate := &authGate{store: store}
		scope, ok := resolveReq(gate, "/mcp", "Bearer "+viewTok)
		if !ok || scope != authtoken.ScopeView {
			t.Fatalf("resolve = (%q, %v), want (view, true)", scope, ok)
		}
	})

	t.Run("scoped token via ?t= query → its tier", func(t *testing.T) {
		gate := &authGate{store: store}
		scope, ok := resolveReq(gate, "/mcp?t="+viewTok, "")
		if !ok || scope != authtoken.ScopeView {
			t.Fatalf("resolve = (%q, %v), want (view, true)", scope, ok)
		}
	})

	t.Run("unknown token → refused even with the open default", func(t *testing.T) {
		// A PRESENT-but-unknown credential (e.g. a revoked session token) must
		// 401, never quietly escalate to the untokened operator default.
		gate := &authGate{store: store}
		if _, ok := resolveReq(gate, "/mcp", "Bearer nope-nope"); ok {
			t.Fatal("unknown bearer resolved; want refusal")
		}
		if _, ok := resolveReq(gate, "/mcp?t=nope-nope", ""); ok {
			t.Fatal("unknown query token resolved; want refusal")
		}
	})

	t.Run("malformed authorization header → refused", func(t *testing.T) {
		gate := &authGate{store: store}
		if _, ok := resolveReq(gate, "/mcp", "Basic dXNlcg=="); ok {
			t.Fatal("malformed Authorization resolved; want refusal")
		}
	})

	t.Run("revocation takes effect on the next request", func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "tokens.json")
		rec, err := authtoken.Mint(path, authtoken.ScopeTriage, "session:gone")
		if err != nil {
			t.Fatalf("mint: %v", err)
		}
		gate := &authGate{store: authtoken.NewStore(path)}
		if _, ok := resolveReq(gate, "/mcp", "Bearer "+rec.Token); !ok {
			t.Fatal("token should resolve before revocation")
		}
		if _, err := authtoken.Revoke(path, rec.Token); err != nil {
			t.Fatalf("revoke: %v", err)
		}
		if _, ok := resolveReq(gate, "/mcp", "Bearer "+rec.Token); ok {
			t.Fatal("revoked token still resolves")
		}
	})
}

// TestAuthGateUntokenedDial pins the -untokened dial's three positions for
// credential-less requests, and that it changes NOTHING else: scoped tokens
// keep their tiers, and a set static token still means "credentials required"
// regardless of the dial.
func TestAuthGateUntokenedDial(t *testing.T) {
	store, viewTok := mintTestToken(t, authtoken.ScopeView)

	t.Run("operator (explicit) → operator, like the zero-value default", func(t *testing.T) {
		gate := &authGate{store: store, untokened: untokenedOperator}
		scope, ok := resolveReq(gate, "/mcp", "")
		if !ok || scope != authtoken.ScopeOperator {
			t.Fatalf("resolve = (%q, %v), want (operator, true)", scope, ok)
		}
	})

	t.Run("view → the read-only tier, with no plugin grants", func(t *testing.T) {
		gate := &authGate{store: store, untokened: untokenedView}
		req := httptest.NewRequest(http.MethodGet, "/mcp", nil)
		rec, ok := gate.resolveRecord(req)
		if !ok || rec.Scope != authtoken.ScopeView {
			t.Fatalf("resolveRecord = (%+v, %v), want a view record", rec, ok)
		}
		if len(rec.Plugins) != 0 {
			t.Fatalf("untokened view record must carry no plugin grants: %+v", rec)
		}
	})

	t.Run("deny → refused", func(t *testing.T) {
		gate := &authGate{store: store, untokened: untokenedDeny}
		if _, ok := resolveReq(gate, "/mcp", ""); ok {
			t.Fatal("credential-less request resolved under deny")
		}
	})

	t.Run("scoped tokens keep their own tier under every dial", func(t *testing.T) {
		for _, mode := range []string{untokenedOperator, untokenedView, untokenedDeny} {
			gate := &authGate{store: store, untokened: mode}
			scope, ok := resolveReq(gate, "/mcp", "Bearer "+viewTok)
			if !ok || scope != authtoken.ScopeView {
				t.Fatalf("mode %s: scoped token resolve = (%q, %v), want (view, true)", mode, scope, ok)
			}
		}
	})

	t.Run("static token overrides the dial: credential-less refused, match is operator", func(t *testing.T) {
		gate := &authGate{static: "s3cret", store: store, untokened: untokenedView}
		if _, ok := resolveReq(gate, "/mcp", ""); ok {
			t.Fatal("static token set: credential-less request must be refused even with -untokened view")
		}
		scope, ok := resolveReq(gate, "/mcp", "Bearer s3cret")
		if !ok || scope != authtoken.ScopeOperator {
			t.Fatalf("static match = (%q, %v), want (operator, true)", scope, ok)
		}
	})
}

func TestCheckUntokenedMode(t *testing.T) {
	for _, ok := range []string{untokenedOperator, untokenedView, untokenedDeny} {
		if err := checkUntokenedMode(ok); err != nil {
			t.Errorf("checkUntokenedMode(%q) = %v, want nil", ok, err)
		}
	}
	// A typo in a lockdown flag must fail startup, not fall back to open.
	for _, bad := range []string{"", "viewer", "OPERATOR", "none"} {
		if err := checkUntokenedMode(bad); err == nil {
			t.Errorf("checkUntokenedMode(%q) should fail", bad)
		}
	}
}

// TestMuxUntokenedDeny proves the deny dial at the HTTP boundary: /mcp 401s a
// bare request while /health stays open and a scoped token still passes.
func TestMuxUntokenedDeny(t *testing.T) {
	client := busclient.New("ws://127.0.0.1:0/bus", "")
	store, viewTok := mintTestToken(t, authtoken.ScopeView)
	gate := &authGate{store: store, untokened: untokenedDeny}
	cache := newServerCache(client, newPluginCatalog(client), tierServers(client))
	mux := newMux(cache, client, gate)
	srv := httptest.NewServer(mux)
	defer srv.Close()

	resp, err := http.Get(srv.URL + "/health")
	if err != nil {
		t.Fatalf("health GET: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("/health status = %d, want 200 (stays open under deny)", resp.StatusCode)
	}

	resp, err = http.Get(srv.URL + "/mcp")
	if err != nil {
		t.Fatalf("mcp GET: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("/mcp bare request status = %d, want 401 under deny", resp.StatusCode)
	}

	req, _ := http.NewRequest(http.MethodGet, srv.URL+"/mcp", nil)
	req.Header.Set("Authorization", "Bearer "+viewTok)
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("mcp GET with scoped token: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized {
		t.Fatalf("/mcp with scoped token was 401 under deny; scoped tokens must still pass")
	}
}

// TestMuxHealthOpenMCPGuarded proves the wiring: /health is reachable without a
// token even when auth is on, while /mcp demands a resolvable credential.
func TestMuxHealthOpenMCPGuarded(t *testing.T) {
	client := busclient.New("ws://127.0.0.1:0/bus", "")
	store, viewTok := mintTestToken(t, authtoken.ScopeView)
	gate := &authGate{static: "s3cret", store: store}
	cache := newServerCache(client, newPluginCatalog(client), tierServers(client))
	mux := newMux(cache, client, gate)
	srv := httptest.NewServer(mux)
	defer srv.Close()

	// /health is open.
	resp, err := http.Get(srv.URL + "/health")
	if err != nil {
		t.Fatalf("health GET: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("/health status = %d, want 200 (must stay open)", resp.StatusCode)
	}

	// /mcp without a credential is rejected before reaching the MCP handler.
	resp, err = http.Get(srv.URL + "/mcp")
	if err != nil {
		t.Fatalf("mcp GET: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("/mcp without token status = %d, want 401", resp.StatusCode)
	}

	// /mcp with the static token passes auth (reaches the MCP handler, which no
	// longer answers 401 — a bare GET is a bad MCP request, so assert not-401).
	req, _ := http.NewRequest(http.MethodGet, srv.URL+"/mcp", nil)
	req.Header.Set("Authorization", "Bearer s3cret")
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("mcp GET with token: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized {
		t.Fatalf("/mcp with correct token was 401; auth should have passed")
	}

	// A scoped token passes auth too, even alongside a static token.
	req, _ = http.NewRequest(http.MethodGet, srv.URL+"/mcp", nil)
	req.Header.Set("Authorization", "Bearer "+viewTok)
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("mcp GET with scoped token: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized {
		t.Fatalf("/mcp with scoped token was 401; auth should have passed")
	}
}

// TestRequireScopeStampsTokenLabel: the gate stamps the resolved record's
// label into the request context so tool handlers (whose servers are CACHED
// and shared across same-grant records) can name the calling token in
// diagnostics — the spawn clamp's strip log reads it via tokenLabelFrom.
// Credential-less requests carry no label and fall back to "untokened".
func TestRequireScopeStampsTokenLabel(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := authtoken.Mint(path, authtoken.ScopeOperator, "session:abc123")
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	gate := &authGate{store: authtoken.NewStore(path)}

	var got string
	h := requireScope(gate, http.HandlerFunc(func(_ http.ResponseWriter, r *http.Request) {
		got = tokenLabelFrom(r.Context())
	}))

	req := httptest.NewRequest(http.MethodPost, "/mcp", nil)
	req.Header.Set("Authorization", "Bearer "+rec.Token)
	h.ServeHTTP(httptest.NewRecorder(), req)
	if got != "session:abc123" {
		t.Fatalf("token label from context = %q, want the record's label", got)
	}

	got = ""
	h.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodPost, "/mcp", nil))
	if got != "untokened" {
		t.Fatalf("credential-less request label = %q, want the untokened fallback", got)
	}
}
