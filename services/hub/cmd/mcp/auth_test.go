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
		name    string
		addr    string
		token   string
		wantErr bool
	}{
		{"loopback ip, no token, allowed", "127.0.0.1:7897", "", false},
		{"loopback name, no token, allowed", "localhost:7897", "", false},
		{"ipv6 loopback, no token, allowed", "[::1]:7897", "", false},
		{"bare port, no token, refused", ":7897", "", true},
		{"all interfaces, no token, refused", "0.0.0.0:7897", "", true},
		{"lan ip, no token, refused", "192.168.1.10:7897", "", true},
		{"lan ip, with token, allowed", "192.168.1.10:7897", "s3cret", false},
		{"all interfaces, with token, allowed", "0.0.0.0:7897", "s3cret", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			err := checkBindPolicy(c.addr, c.token)
			if (err != nil) != c.wantErr {
				t.Fatalf("checkBindPolicy(%q, tokenSet=%v) err = %v, wantErr = %v", c.addr, c.token != "", err, c.wantErr)
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
