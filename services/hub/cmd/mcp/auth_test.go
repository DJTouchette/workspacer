package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
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

	t.Run("no credential, no static token → refused (the zero value fails closed)", func(t *testing.T) {
		// This used to be operator. It is deny now, and the zero value is what
		// a future authGate built without the field would carry — so the
		// unconfigured shape must be the SAFE one, not the open one.
		gate := &authGate{store: store}
		if scope, ok := resolveReq(gate, "/mcp", ""); ok {
			t.Fatalf("resolve = (%q, true), want refusal: an authGate with no dial set must not grant %s", scope, scope)
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

	t.Run("unknown token → refused even with the open dial", func(t *testing.T) {
		// A PRESENT-but-unknown credential (e.g. a revoked session token) must
		// 401, never quietly escalate to the untokened tier. Dialled explicitly
		// to operator so this still tests the escalation and not merely the
		// shipped deny.
		gate := &authGate{store: store, untokened: untokenedOperator}
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

	t.Run("operator (explicit, opt-in) → operator", func(t *testing.T) {
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

	// The credential-less fallback label is exercised through a gate that
	// deliberately admits such callers (-untokened operator); under the shipped
	// deny they never reach a handler at all, which the next assertion pins.
	got = ""
	open := &authGate{store: authtoken.NewStore(path), untokened: untokenedOperator}
	requireScope(open, http.HandlerFunc(func(_ http.ResponseWriter, r *http.Request) {
		got = tokenLabelFrom(r.Context())
	})).ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodPost, "/mcp", nil))
	if got != "untokened" {
		t.Fatalf("credential-less request label = %q, want the untokened fallback", got)
	}

	reached := false
	h = requireScope(gate, http.HandlerFunc(func(_ http.ResponseWriter, _ *http.Request) { reached = true }))
	rec2 := httptest.NewRecorder()
	h.ServeHTTP(rec2, httptest.NewRequest(http.MethodPost, "/mcp", nil))
	if reached || rec2.Code != http.StatusUnauthorized {
		t.Fatalf("credential-less request reached=%v status=%d, want refused with 401 under the shipped default", reached, rec2.Code)
	}
}

// TestUntokenedDefaultDeniesFleetControl is the release blocker's regression
// pin: the facade must not hand fleet control to a caller that presents no
// credential at all.
//
// It fails if anyone restores the old untokened-operator default — whether by
// flipping the constant, by rewriting the dial's default resolution, by making
// the authGate zero value permissive again, or by relaxing the HTTP gate. Each
// of those four is asserted separately so the failure names the mistake.
//
// The reason this can be strict: NOTHING legitimate is credential-less. Every
// facade session the desktop (claudeSpawn.ts / managedSpawn.ts) or the brain
// (cmd/brain/facade.go) spawns mints a per-session tokens.json record and
// presents it — as an Authorization header on the --mcp-config file for
// PTY/claude-stream, as a ?t= query param for the URL-only codex/opencode/
// copilot registrations. Only a hand-configured client was ever untokened, and
// it can mint its own with `workspacer token create`.
func TestUntokenedDefaultDeniesFleetControl(t *testing.T) {
	// 1. The shipped constant.
	if defaultUntokened == untokenedOperator {
		t.Fatal("defaultUntokened is operator: a local process with no credential can spawn agents, write files and rewrite config. It must be deny.")
	}
	if defaultUntokened != untokenedDeny {
		t.Fatalf("defaultUntokened = %q, want %q — view still serves tools (and every transcript) to an uncredentialed caller", defaultUntokened, untokenedDeny)
	}

	// 2. The default resolution the flag actually uses, env unset.
	t.Setenv("WKS_MCP_UNTOKENED", "")
	if got := untokenedDefault(); got != untokenedDeny {
		t.Fatalf("untokenedDefault() = %q with WKS_MCP_UNTOKENED unset, want %q", got, untokenedDeny)
	}
	// The env override still works — the lockdown is a default, not a wall.
	t.Setenv("WKS_MCP_UNTOKENED", untokenedOperator)
	if got := untokenedDefault(); got != untokenedOperator {
		t.Fatalf("untokenedDefault() = %q with WKS_MCP_UNTOKENED=operator, want the override to win", got)
	}

	// 3. The gate, both at the shipped default and at the zero value.
	store, sessionTok := mintTestToken(t, authtoken.ScopeOperator)
	for name, gate := range map[string]*authGate{
		"shipped default": {store: store, untokened: defaultUntokened},
		"zero value":      {store: store},
	} {
		if scope, ok := resolveReq(gate, "/mcp", ""); ok {
			t.Fatalf("%s: a credential-less request resolved to %q; want refusal", name, scope)
		}
		// …and the session tokens the desktop mints still work, which is the
		// whole reason denying is affordable.
		if scope, ok := resolveReq(gate, "/mcp", "Bearer "+sessionTok); !ok || scope != authtoken.ScopeOperator {
			t.Fatalf("%s: per-session token resolve = (%q, %v), want (operator, true)", name, scope, ok)
		}
	}

	// 4. The HTTP boundary, wired exactly as main() wires it: no tools reach an
	// uncredentialed caller, and /health stays open.
	client := busclient.New("ws://127.0.0.1:0/bus", "")
	// defaultUntokened directly, not untokenedDefault(): this step tests the
	// SHIPPED configuration, not whatever WKS_MCP_UNTOKENED the subtest above
	// left installed.
	gate := &authGate{store: store, untokened: defaultUntokened}
	mux := newMux(newServerCache(client, newPluginCatalog(client), tierServers(client)), client, gate)
	srv := httptest.NewServer(servedHandler("127.0.0.1:7897", mux))
	defer srv.Close()

	for _, path := range []string{"/mcp", "/sse"} {
		resp, err := http.Post(srv.URL+path, "application/json",
			strings.NewReader(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`))
		if err != nil {
			t.Fatalf("POST %s: %v", path, err)
		}
		body, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		if resp.StatusCode != http.StatusUnauthorized {
			t.Fatalf("POST %s uncredentialed = %d, want 401. Body: %s", path, resp.StatusCode, body)
		}
		if strings.Contains(string(body), "spawn_agent") {
			t.Fatalf("POST %s uncredentialed leaked the operator tool surface: %s", path, body)
		}
	}

	resp, err := http.Get(srv.URL + "/health")
	if err != nil {
		t.Fatalf("health GET: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("/health = %d, want 200 — liveness probes must not need a secret", resp.StatusCode)
	}
}
