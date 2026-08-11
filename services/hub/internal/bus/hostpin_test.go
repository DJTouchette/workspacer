package bus

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// withLocalAddr attaches the address a request's socket "landed on", which is
// what separates a rebind from a legitimate remote call.
func withLocalAddr(r *http.Request, addr string) *http.Request {
	if addr == "" {
		return r
	}
	tcp, err := net.ResolveTCPAddr("tcp", addr)
	if err != nil {
		panic(err)
	}
	return r.WithContext(context.WithValue(r.Context(), http.LocalAddrContextKey, net.Addr(tcp)))
}

func hostPinStatus(t *testing.T, host, localAddr string) int {
	t.Helper()
	h := (&Server{}).requireHost(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusTeapot) // a status nothing else here returns
	}))
	req := httptest.NewRequest(http.MethodGet, "/plugins", nil)
	req.Host = host
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, withLocalAddr(req, localAddr))
	return rec.Code
}

// THE REBINDING SHAPE. The hub was the only one of the three HTTP servers with
// no Host pin — claudemon has one on both routers, the MCP facade wraps its
// whole mux — and the measured consequence was that `Host: evil.example.com`
// got 403 from /bus and 200 plus the full plugin manifest from /plugins on the
// same hub, because only /bus was rebinding-aware.
func TestHostPinRefusesAForeignHostOnALoopbackSocket(t *testing.T) {
	for _, host := range []string{
		"evil.example.com",
		"evil.example.com:7895",
		"attacker.tld:80",
		"127.0.0.1.evil.tld:7895", // a name that merely starts like loopback
		"10.0.0.9:7895",           // a private address that is not the one we bound
	} {
		if got := hostPinStatus(t, host, "127.0.0.1:7895"); got != http.StatusForbidden {
			t.Errorf("Host %q on the loopback listener: got %d, want 403", host, got)
		}
	}
}

func TestHostPinAllowsLoopbackAndTheAddressTheSocketLandedOn(t *testing.T) {
	for _, tc := range []struct{ host, local string }{
		{"127.0.0.1:7895", "127.0.0.1:7895"},
		{"127.0.0.1", "127.0.0.1:7895"},
		{"127.0.0.2:7895", "127.0.0.1:7895"}, // all of 127/8 is this machine
		{"localhost:7895", "127.0.0.1:7895"},
		{"LOCALHOST:7895", "127.0.0.1:7895"},
		{"[::1]:7895", "127.0.0.1:7895"},
		{"", "127.0.0.1:7895"}, // HTTP/1.0 and local probes send no Host
		// The remote-share deployment: the socket landed on the tailnet
		// address, so the name it was reached by is legitimate. Refusing this
		// would mean enumerating names the hub cannot know.
		{"box.tailnet.ts.net:7895", "100.86.79.73:7895"},
		{"100.86.79.73:7895", "100.86.79.73:7895"},
	} {
		if got := hostPinStatus(t, tc.host, tc.local); got != http.StatusTeapot {
			t.Errorf("Host %q on socket %s: got %d, want pass-through", tc.host, tc.local, got)
		}
	}
}

// The pin has to be applied to the WHOLE mux, not to the routes someone
// remembered: the leak was on /plugins, and /bus was the one route already
// covered. Driven through the real Handler() over a real loopback listener.
func TestHostPinCoversEveryRouteTheHubServes(t *testing.T) {
	srv := NewServer(broker.New())
	srv.AddRoute("/plugins", func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("MANIFESTS"))
	})
	ts := httptest.NewServer(srv.Handler())
	defer ts.Close()

	for _, path := range []string{"/plugins", "/health", "/bus"} {
		req, err := http.NewRequest(http.MethodGet, ts.URL+path, nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Host = "evil.example.com"
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatalf("%s: %v", path, err)
		}
		resp.Body.Close()
		if resp.StatusCode != http.StatusForbidden {
			t.Errorf("GET %s with a rebound Host: got %d, want 403", path, resp.StatusCode)
		}
	}
	// …and the same routes still answer a loopback caller, or the pin is an
	// outage rather than a boundary.
	resp, err := http.Get(ts.URL + "/plugins")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Errorf("loopback GET /plugins: got %d, want 200", resp.StatusCode)
	}
}

// AuthorizedForPlugin is the gate on the settings block injected into a
// plugin's HTML document. It must admit the host and the plugin's OWN token,
// and nothing else — a second plugin's token is a different plugin's business.
func TestAuthorizedForPluginAdmitsOnlyTheHostAndThatPluginsOwnToken(t *testing.T) {
	srv := NewServer(broker.New())
	srv.SetToken("HOSTTOKEN")
	srv.RegisterPluginToken("PLUG-A", "acme.a", []capspec.Grant{{Method: "agents.list"}}, capspec.EventGrants{})
	srv.RegisterPluginToken("PLUG-B", "acme.b", []capspec.Grant{{Method: "agents.list"}}, capspec.EventGrants{})

	get := func(query string, header string) *http.Request {
		r := httptest.NewRequest(http.MethodGet, "/plugins/ui/acme.a/"+query, nil)
		if header != "" {
			r.Header.Set("Authorization", "Bearer "+header)
		}
		return r
	}
	cases := []struct {
		name string
		req  *http.Request
		want bool
	}{
		{"anonymous", get("", ""), false},
		{"wrong token", get("", "NOPE"), false},
		{"host token", get("", "HOSTTOKEN"), true},
		{"own token as ?busToken", get("?busToken=PLUG-A", ""), true},
		{"own token as ?token", get("?token=PLUG-A", ""), true},
		{"own token as bearer", get("", "PLUG-A"), true},
		{"another plugin's token", get("?busToken=PLUG-B", ""), false},
	}
	for _, tc := range cases {
		if got := srv.AuthorizedForPlugin(tc.req, "acme.a"); got != tc.want {
			t.Errorf("%s: AuthorizedForPlugin = %v, want %v", tc.name, got, tc.want)
		}
	}
	// A revoked plugin token stops being an entitlement immediately — the same
	// property the bus already holds for live connections.
	srv.UnregisterPluginToken("PLUG-A")
	if srv.AuthorizedForPlugin(get("?busToken=PLUG-A", ""), "acme.a") {
		t.Error("a revoked plugin token still unlocked that plugin's settings")
	}
	// With no host token configured (the loopback default) Authorized is true
	// for everyone, and this must not become stricter than the thing it wraps.
	open := NewServer(broker.New())
	if !open.AuthorizedForPlugin(get("", ""), "acme.a") {
		t.Error("with no token configured, AuthorizedForPlugin refused a caller Authorized would admit")
	}
}
