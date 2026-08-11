package bus

// A REVERSE PROXY IN FRONT OF THE HUB is a supported, shipped configuration:
// the desktop's Remote Share dialog has a one-tap "HTTPS via Tailscale" toggle
// that runs `tailscale serve --bg <hubPort>`, and that HTTPS front is the only
// way the /m PWA gets the secure context Web Push requires.
//
// `tailscale serve` terminates TLS for <node>.ts.net and forwards to the hub's
// LOOPBACK socket, so the hub sees a non-loopback Host on a connection that
// landed on 127.0.0.1 — byte-for-byte the DNS-rebinding shape requireHost and
// originAllowed refuse. The guards are right and cannot tell the two apart from
// the request alone, so the operator names the proxy (hub --trusted-host, set
// by the desktop when it enables the toggle). These cases pin BOTH pins: a fix
// to requireHost alone still leaves /bus 403 for every browser client.

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
)

const tailnetName = "djtouchette.tail65dbc5.ts.net"

func proxiedRequest(path, host, origin string) *http.Request {
	r := httptest.NewRequest(http.MethodGet, "http://"+host+path, nil)
	r.Host = host
	if origin != "" {
		r.Header.Set("Origin", origin)
	}
	// The proxy runs on this machine, so the hub's socket landed on loopback.
	return r.WithContext(context.WithValue(r.Context(), http.LocalAddrContextKey,
		net.Addr(&net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 7895})))
}

func TestReverseProxyHostIs403WithoutBeingDeclared(t *testing.T) {
	s := &Server{}
	if s.hostAllowed(proxiedRequest("/m", tailnetName, "")) {
		t.Fatal("an undeclared foreign Host on a loopback socket must stay refused — that is the rebinding shape")
	}
	if s.originAllowed(proxiedRequest("/bus", tailnetName, "https://"+tailnetName)) {
		t.Fatal("an undeclared foreign same-origin upgrade on a loopback socket must stay refused")
	}
}

func TestDeclaredReverseProxyHostPassesBothPins(t *testing.T) {
	s := &Server{}
	s.SetTrustedHosts([]string{tailnetName})

	for _, path := range []string{"/health", "/m", "/plugins", "/app/", "/bus"} {
		if !s.hostAllowed(proxiedRequest(path, tailnetName, "")) {
			t.Errorf("%s: the declared proxy host was refused by the Host pin — every route behind `tailscale serve` 403s", path)
		}
	}
	// The browser half: the PWA is SERVED from https://<node>.ts.net, so its
	// /bus upgrade carries that Origin and the same Host.
	if !s.originAllowed(proxiedRequest("/bus", tailnetName, "https://"+tailnetName)) {
		t.Error("the declared proxy host was refused by the Origin pin — the web/mobile clients can never open the bus")
	}
	// Case-insensitively, and with a port on either side.
	if !s.hostAllowed(proxiedRequest("/health", "DJTouchette.Tail65dbc5.TS.net:443", "")) {
		t.Error("host matching must be case-insensitive and port-agnostic")
	}
}

func TestDeclaringOneProxyHostDoesNotWidenToAnyOther(t *testing.T) {
	s := &Server{}
	s.SetTrustedHosts([]string{tailnetName})

	if s.hostAllowed(proxiedRequest("/plugins", "evil.example.com", "")) {
		t.Error("declaring one proxy host must not admit any other Host")
	}
	// A cross-SITE page dialing the declared host is still a cross-site page.
	if s.originAllowed(proxiedRequest("/bus", tailnetName, "https://evil.example.com")) {
		t.Error("a cross-site Origin against the declared host must stay refused")
	}
}

// A wildcard would re-open every route to any page that can resolve a name to
// 127.0.0.1, so it must never be accepted as a name.
func TestTrustedHostsRefusesAWildcard(t *testing.T) {
	s := &Server{}
	s.SetTrustedHosts([]string{"*", "", "   "})
	if s.hostAllowed(proxiedRequest("/plugins", "evil.example.com", "")) {
		t.Fatal(`SetTrustedHosts accepted "*" — that is an allow-anything Host pin`)
	}
	if len(s.trustedHosts) != 0 {
		t.Fatalf("expected no usable trusted hosts, got %v", s.trustedHosts)
	}
}

// The exemption is opt-in: a hub told nothing behaves exactly as before.
func TestNoTrustedHostsMeansTodaysShapeOnlyRule(t *testing.T) {
	s := &Server{}
	if s.hostAllowed(proxiedRequest("/plugins", "evil.example.com", "")) {
		t.Fatal("with no trusted hosts the rebinding shape must still be refused")
	}
	if !s.hostAllowed(proxiedRequest("/plugins", "127.0.0.1:7895", "")) {
		t.Fatal("loopback must still be allowed")
	}
}

// End to end through the real Handler(), on a real loopback listener: the
// composition, not just the predicate. This is the measured symptom — every
// route behind `tailscale serve` answering 403 — reproduced in-process.
func TestEveryRouteBehindADeclaredProxyAnswersInsteadOf403(t *testing.T) {
	s := NewServer(broker.New())
	s.AddRoute("/plugins", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusTeapot)
	})
	srv := httptest.NewServer(s.Handler())
	defer srv.Close()

	get := func(path string) int {
		req, err := http.NewRequest(http.MethodGet, srv.URL+path, nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Host = tailnetName // what `tailscale serve` forwards
		res, err := srv.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer res.Body.Close()
		return res.StatusCode
	}

	if got := get("/health"); got != http.StatusForbidden {
		t.Fatalf("control: an undeclared proxy host should still be 403, got %d", got)
	}

	s.SetTrustedHosts([]string{tailnetName})
	for _, path := range []string{"/health", "/plugins"} {
		if got := get(path); got == http.StatusForbidden {
			t.Errorf("%s answered 403 behind the declared proxy — the shipped `tailscale serve` toggle kills the whole plane", path)
		}
	}
}
