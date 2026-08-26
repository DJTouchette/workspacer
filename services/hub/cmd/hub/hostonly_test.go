package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"regexp"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

// THE NODE-TOKEN REFUSAL, end to end through the wrapper the routes are
// registered with.
//
// A remote worker node holds an operator-tier scoped token (deploy/fly/node,
// `workspacer token create --scope operator`). That token is trusted on the bus
// and therefore passes guard() on every /plugins/* route — and POST
// /plugins/install runs the manifest's install argv on THIS host. These tests
// pin the interim gate: the install family answers a node's token with 403 and
// never reaches the handler, while the user's own local install (the host
// token, which is what the desktop's hubAuthHeaders presents) still runs.

// hostOnlyFixture builds a server with the host token and a node-style
// operator-tier scoped token, plus a sentinel handler that records whether the
// guarded work ran. Refusing with the right status while still executing the
// handler would be no refusal at all, so `ran` is the assertion that matters.
func hostOnlyFixture(t *testing.T) (srv *bus.Server, h http.HandlerFunc, ran *bool) {
	t.Helper()
	srv = bus.NewServer(broker.New())
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (bus.ScopedIdent, bool) {
		switch tok {
		case "node-token":
			return bus.ScopedIdent{Scope: "operator", Methods: []string{"*"}, Label: "fly-node"}, true
		case "phone-token":
			return bus.ScopedIdent{Scope: "triage", Methods: []string{"claude.approve"}}, true
		}
		return bus.ScopedIdent{}, false
	})
	executed := false
	ran = &executed
	h = hostOnlyRoute(srv, "plugin install", func(w http.ResponseWriter, _ *http.Request) {
		executed = true
		w.WriteHeader(http.StatusOK)
		_ = json.NewEncoder(w).Encode(map[string]string{"id": "acme.plugin"})
	})
	return srv, h, ran
}

func postInstall(h http.HandlerFunc, tok string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(http.MethodPost, "/plugins/install", strings.NewReader(`{"url":"https://github.com/acme/plugin"}`))
	if tok != "" {
		req.Header.Set("Authorization", "Bearer "+tok)
	}
	rec := httptest.NewRecorder()
	h(rec, req)
	return rec
}

// TestHostOnlyRouteRefusesANodesOperatorToken is the fix, stated as the attack:
// the credential that sits in a Fly secret on a remote machine cannot make this
// host download and build a plugin.
func TestHostOnlyRouteRefusesANodesOperatorToken(t *testing.T) {
	srv, h, ran := hostOnlyFixture(t)

	// Control: the node's token IS authorized on this hub — that is the whole
	// problem, and if this ever fails the test below proves nothing.
	authReq := httptest.NewRequest(http.MethodPost, "/plugins/install", nil)
	authReq.Header.Set("Authorization", "Bearer node-token")
	if !srv.Authorized(authReq) {
		t.Fatal("fixture drift: the node token must pass srv.Authorized, or this test is not exercising the gap")
	}

	rec := postInstall(h, "node-token")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("install with a node's operator token = %d, want 403: this token can run arbitrary code on the hub host through the install argv", rec.Code)
	}
	if *ran {
		t.Fatal("the install handler RAN for a node token — the status code was cosmetic and the plugin was installed anyway")
	}
	// The refusal must explain itself; it must not name the credential (a caller
	// that can enumerate labels can go looking for one to steal).
	body := rec.Body.String()
	if !strings.Contains(body, "host authority") {
		t.Errorf("refusal body %q says nothing about host authority — an operator reading a client log has no idea what to do", body)
	}
	if strings.Contains(body, "fly-node") || strings.Contains(body, "node-token") {
		t.Errorf("refusal body %q names the presented credential; that belongs in the hub's log, not in the answer", body)
	}
}

// TestHostOnlyRouteStillLetsTheUsersOwnInstallThrough: the host token is what
// the desktop (hubAuthHeaders → getHubToken → <config>/workspacer/remote-token),
// the CLI and `workspacer plugin dev` present. Local plugin install must be
// untouched by this gate.
func TestHostOnlyRouteStillLetsTheUsersOwnInstallThrough(t *testing.T) {
	_, h, ran := hostOnlyFixture(t)
	rec := postInstall(h, "host-secret")
	if rec.Code != http.StatusOK {
		t.Fatalf("install with the HOST token = %d, want 200 — the user's own plugin install has been broken by the node gate", rec.Code)
	}
	if !*ran {
		t.Fatal("the host token was accepted but the handler never ran")
	}
}

// TestHostOnlyRouteKeepsThe401ForEveryoneElse: the gate ADDS a refusal, it does
// not replace the token check. A phone-tier token and an anonymous caller must
// still get 401, not the more informative 403 — 403 is reserved for a caller
// whose credential is real and simply not the host's.
func TestHostOnlyRouteKeepsThe401ForEveryoneElse(t *testing.T) {
	_, h, ran := hostOnlyFixture(t)
	for _, tok := range []string{"", "phone-token", "bogus"} {
		rec := postInstall(h, tok)
		if rec.Code != http.StatusUnauthorized {
			t.Errorf("install with token %q = %d, want 401", tok, rec.Code)
		}
		if *ran {
			t.Fatalf("the install handler ran for token %q", tok)
		}
	}
}

// TestHostOnlyRouteIsOpenOnAnUnauthenticatedHub pins the loopback default: with
// no token configured there is no host credential to present, Authorized already
// answers true for everyone, and a gate that refused would break local dev.
func TestHostOnlyRouteIsOpenOnAnUnauthenticatedHub(t *testing.T) {
	srv := bus.NewServer(broker.New())
	ran := false
	h := hostOnlyRoute(srv, "plugin install", func(w http.ResponseWriter, _ *http.Request) {
		ran = true
		w.WriteHeader(http.StatusOK)
	})
	if rec := postInstall(h, ""); rec.Code != http.StatusOK || !ran {
		t.Fatalf("no-token hub: code=%d ran=%v, want 200 + handler run", rec.Code, ran)
	}
}

// TestTheInstallFamilyIsRegisteredHostOnly is the wiring bearing: a perfect
// hostOnlyRoute that main() never wraps a route in protects nothing, and the
// three routes here are the three that end in a process running on this host.
// capspec's HTTP-plane guard pins the same three from the registry side; this
// one reads main.go directly so the pair cannot be satisfied by editing only
// the classification.
func TestTheInstallFamilyIsRegisteredHostOnly(t *testing.T) {
	src := readSource(t, "main.go")
	for _, route := range []string{"/plugins/install", "/plugins/examples/install", "/plugins/reload"} {
		re := regexp.MustCompile(`AddRoute\(\s*"` + regexp.QuoteMeta(route) + `"\s*,\s*hostOnly\(`)
		if !re.MatchString(src) {
			t.Errorf("main.go does not register %q with hostOnly(...). That route runs code on the hub's own machine, and guard() alone admits the operator-tier scoped token every remote worker node carries.", route)
		}
	}
	// And the wrapper must still be the composed one — a hostOnly that forgot
	// srv.Authorized would turn 401s into 403s and admit view/triage tiers.
	guardSrc := readSource(t, "hostonly.go")
	for _, want := range []string{"srv.Authorized(r)", "srv.HostAuthorized(r)"} {
		if !strings.Contains(guardSrc, want) {
			t.Errorf("hostonly.go no longer calls %s — the gate is half a check", want)
		}
	}
}

// readSource reads a file from this package's own directory, so the two
// bearings above fail loudly if the file is renamed rather than passing on an
// empty string.
func readSource(t *testing.T, name string) string {
	t.Helper()
	b, err := os.ReadFile(name)
	if err != nil {
		t.Fatalf("read %s: %v", name, err)
	}
	return string(b)
}
