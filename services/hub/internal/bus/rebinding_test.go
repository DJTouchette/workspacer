package bus

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"
)

// TestOriginAllowedRejectsDNSRebinding models a DNS-rebinding page: the browser
// still believes it is talking to the public name it was served from, so it
// reports Origin == Host == that public name. But the attacker has rebound that
// name to 127.0.0.1, so the socket actually terminates at loopback. originAllowed
// must NOT trust the reflected Host in that case — same-origin against a public
// Host while the connection lands on loopback is precisely the vector this check
// exists to stop. Covers idx 18.
func TestOriginAllowedRejectsDNSRebinding(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "http://evil.example:7895/bus", nil)
	r.Host = "evil.example:7895"
	r.Header.Set("Origin", "http://evil.example:7895")
	// Model where the socket really landed: loopback (the rebound A record).
	ctx := context.WithValue(r.Context(), http.LocalAddrContextKey,
		&net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 7895})
	r = r.WithContext(ctx)

	if originAllowed(r) {
		t.Fatal("DNS-rebinding page (public Origin==Host, loopback socket) must be rejected")
	}

	// Sanity: the legitimate case — a connection that genuinely terminates at the
	// public address the browser dialed (e.g. the hub-served Tailscale remote UI)
	// is still same-origin and must be allowed.
	ctx = context.WithValue(r.Context(), http.LocalAddrContextKey,
		&net.TCPAddr{IP: net.IPv4(100, 64, 0, 5), Port: 7895})
	r = r.WithContext(ctx)
	if !originAllowed(r) {
		t.Fatal("genuine non-loopback same-origin connection must be allowed")
	}
}
