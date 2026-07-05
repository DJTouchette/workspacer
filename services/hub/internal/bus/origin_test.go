package bus

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
)

// dialOrigin attempts a /bus handshake presenting the given Origin header (empty
// = send none, i.e. a non-browser client). Returns nil on a successful upgrade.
func dialOrigin(t *testing.T, httpURL, origin string) error {
	t.Helper()
	wsURL := strings.Replace(httpURL, "http://", "ws://", 1) + "/bus"
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	var opts *websocket.DialOptions
	if origin != "" {
		opts = &websocket.DialOptions{HTTPHeader: http.Header{"Origin": {origin}}}
	}
	c, _, err := websocket.Dial(ctx, wsURL, opts)
	if err == nil {
		c.CloseNow()
	}
	return err
}

func TestBusOriginPolicy(t *testing.T) {
	srv := NewServer(broker.New())
	hs := httptest.NewServer(srv.Handler())
	defer hs.Close()
	// hs.URL is http://127.0.0.1:PORT; the request Host is that host:port.
	host := strings.TrimPrefix(hs.URL, "http://")

	cases := []struct {
		name   string
		origin string
		allow  bool
	}{
		{"no-origin native client", "", true},
		{"same-origin (hub-served remote UI)", "http://" + host, true},
		{"same-origin https scheme", "https://" + host, true},
		{"loopback localhost other port", "http://localhost:5173", true},
		{"loopback 127.0.0.1 other port", "http://127.0.0.1:65000", true},
		{"cross-site public origin", "http://evil.example.com", false},
		{"cross-site https origin", "https://attacker.test", false},
		{"opaque null origin", "null", false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := dialOrigin(t, hs.URL, tc.origin)
			if tc.allow && err != nil {
				t.Fatalf("origin %q: expected accept, got error: %v", tc.origin, err)
			}
			if !tc.allow && err == nil {
				t.Fatalf("origin %q: expected reject, but handshake succeeded", tc.origin)
			}
		})
	}
}

// A Tailscale-style hostname the page is served from is same-origin: the Origin
// host equals the Host the client dialed, so it must be allowed even though it's
// neither loopback nor the httptest bind address.
func TestOriginAllowedTailscaleSameHost(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "http://myhost.tailnet.ts.net:7895/bus", nil)
	r.Host = "myhost.tailnet.ts.net:7895"
	r.Header.Set("Origin", "http://myhost.tailnet.ts.net:7895")
	if !originAllowed(r) {
		t.Fatal("same-host Tailscale origin should be allowed")
	}
	// A different remote origin dialing that same host is a cross-site page.
	r.Header.Set("Origin", "http://evil.tailnet.ts.net:7895")
	if originAllowed(r) {
		t.Fatal("cross-site origin against the Tailscale host should be rejected")
	}
}
