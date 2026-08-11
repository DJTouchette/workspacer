package main

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func guarded(bindAddr string) http.Handler {
	return requireHost(bindAddr, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusTeapot) // a status nothing else here returns
	}))
}

func status(t *testing.T, h http.Handler, host string) int {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/mcp", nil)
	req.Host = host
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec.Code
}

// The rebinding case this exists for: the attacker's name resolves to
// 127.0.0.1, so the browser treats the request as same-origin and sends it with
// no preflight — but the Host header still says who was dialed.
func TestHostGuardRejectsForeignHost(t *testing.T) {
	h := guarded("127.0.0.1:7897")
	for _, host := range []string{
		"evil.example.com",
		"evil.example.com:7897",
		"attacker.tld:80",
		"127.0.0.1.evil.tld:7897", // a name that merely starts like loopback
	} {
		if got := status(t, h, host); got != http.StatusForbidden {
			t.Errorf("Host %q: got %d, want 403", host, got)
		}
	}
}

func TestHostGuardAllowsLoopback(t *testing.T) {
	h := guarded("127.0.0.1:7897")
	for _, host := range []string{
		"127.0.0.1:7897",
		"127.0.0.1",
		"127.0.0.2:7897", // all of 127.0.0.0/8 is this machine
		"localhost:7897",
		"LOCALHOST:7897",
		"[::1]:7897",
		"", // HTTP/1.0 and some local probes send no Host
	} {
		if got := status(t, h, host); got != http.StatusTeapot {
			t.Errorf("Host %q: got %d, want pass-through", host, got)
		}
	}
}

// A deliberate non-loopback bind (an operator running the facade for a tailnet,
// which the bind policy already forces a token for) must still accept its own
// name, or the guard would break the very deployment it was configured for.
func TestHostGuardAllowsTheConfiguredBindHost(t *testing.T) {
	h := guarded("100.64.0.2:7897")
	if got := status(t, h, "100.64.0.2:7897"); got != http.StatusTeapot {
		t.Errorf("configured bind host: got %d, want pass-through", got)
	}
	if got := status(t, h, "evil.example.com"); got != http.StatusForbidden {
		t.Errorf("foreign host on a bound facade: got %d, want 403", got)
	}
}

// A wildcard bind names no host, so it must not widen the allowlist to
// something an attacker could match.
func TestHostGuardWildcardBindAddsNothing(t *testing.T) {
	for _, bind := range []string{"0.0.0.0:7897", "[::]:7897"} {
		h := guarded(bind)
		if got := status(t, h, "evil.example.com"); got != http.StatusForbidden {
			t.Errorf("bind %q: foreign host got %d, want 403", bind, got)
		}
		if got := status(t, h, "127.0.0.1:7897"); got != http.StatusTeapot {
			t.Errorf("bind %q: loopback got %d, want pass-through", bind, got)
		}
	}
}

// COMPOSITION, not the predicate. Every case above builds its own guarded()
// helper, which proves requireHost works and never that main() applies it — so
// serving `mux` bare (the whole rebinding defense gone) passed the entire
// package. servedHandler is the one thing main hands to http.Server, so this
// exercises the same value.
func TestTheHandlerTheFacadeServesIsHostPinned(t *testing.T) {
	inner := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusTeapot)
	})
	h := servedHandler("127.0.0.1:7897", inner)

	if got := status(t, h, "evil.example.com"); got != http.StatusForbidden {
		t.Fatalf("the served handler let a foreign Host through (%d): on the loopback default this pin is the ONLY thing between a web page and the fleet", got)
	}
	if got := status(t, h, "127.0.0.1:7897"); got != http.StatusTeapot {
		t.Fatalf("the served handler refused loopback (%d)", got)
	}
	// /health included, deliberately.
	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	req.Host = "evil.example.com"
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("/health is not behind the pin (%d)", rec.Code)
	}
}

// The wildcard-normalisation branch, exercised with THE WILDCARD ITSELF as the
// Host. Without those three rows the branch could be deleted and nothing went
// red: "evil.example.com" is refused either way (it never equals extra) and
// "127.0.0.1:7897" is allowed either way (loopback short-circuits first).
// `http://0.0.0.0:<port>` is reachable from a browser on Linux and macOS and
// resolves to loopback, so a widened allowlist is a live same-origin path in.
func TestHostGuardWildcardBindDoesNotAllowTheWildcardItselfAsAHost(t *testing.T) {
	cases := []struct{ bind, host string }{
		{"0.0.0.0:7897", "0.0.0.0:7897"},
		{"0.0.0.0:7897", "0.0.0.0"},
		{"[::]:7897", "[::]:7897"},
		{"[::]:7897", "::"},
	}
	for _, tc := range cases {
		if got := status(t, guarded(tc.bind), tc.host); got != http.StatusForbidden {
			t.Errorf("bind %q + Host %q: got %d, want 403 — the wildcard widened the allowlist to a name a page can dial", tc.bind, tc.host, got)
		}
	}
}
