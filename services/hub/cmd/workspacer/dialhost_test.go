package main

// `workspacer serve --host <concrete address>` — the tailnet form the --host
// help text itself recommends — must be able to boot. The ready-wait probes the
// hub we just spawned, and a concretely-bound listener does NOT answer on
// loopback: a hardcoded 127.0.0.1 health URL tore the whole stack down 20s
// after a healthy start and reported "hub failed to become healthy", pointing
// the operator at the one component that was working. 0.0.0.0 masks it because
// a wildcard bind does answer on loopback.

import (
	"net"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
)

func TestHubHealthProbeFollowsTheBindHost(t *testing.T) {
	cases := []struct {
		host string
		want string
	}{
		{"127.0.0.1", "http://127.0.0.1:7895/health"},
		{"0.0.0.0", "http://127.0.0.1:7895/health"},         // wildcard names no host
		{"::", "http://127.0.0.1:7895/health"},              // ditto, IPv6
		{"", "http://127.0.0.1:7895/health"},                // unset
		{"100.86.79.73", "http://100.86.79.73:7895/health"}, // tailnet IP
		{"192.168.1.66", "http://192.168.1.66:7895/health"}, // LAN IP
		{"fd7a:115c:a1e0::1", "http://[fd7a:115c:a1e0::1]:7895/health"},
	}
	for _, tc := range cases {
		p := buildServePlan(serveOptions{Host: tc.host, HubPort: 7895, APIPort: 7891})
		if p.HubHealth != tc.want {
			t.Errorf("--host %q: HubHealth = %q, want %q — serve waits on an address the hub never bound and then blames the hub",
				tc.host, p.HubHealth, tc.want)
		}
	}
}

// The probe URL is not decoration: it must actually reach a server bound to the
// concrete host. Bind a real listener on a non-loopback local address and check
// that the plan's URL is the one that answers.
func TestHubHealthProbeReachesAConcretelyBoundListener(t *testing.T) {
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		t.Skip("no interface list available")
	}
	var host string
	for _, a := range addrs {
		ipnet, ok := a.(*net.IPNet)
		if !ok || ipnet.IP.IsLoopback() {
			continue
		}
		if v4 := ipnet.IP.To4(); v4 != nil {
			host = v4.String()
			break
		}
	}
	if host == "" {
		t.Skip("no non-loopback IPv4 address on this machine")
	}

	ln, err := net.Listen("tcp", net.JoinHostPort(host, "0"))
	if err != nil {
		t.Skipf("cannot bind %s: %v", host, err)
	}
	srv := &httptest.Server{
		Listener: ln,
		Config:   &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK) })},
	}
	srv.Start()
	defer srv.Close()

	_, portStr, _ := net.SplitHostPort(ln.Addr().String())
	port, _ := strconv.Atoi(portStr)
	p := buildServePlan(serveOptions{Host: host, HubPort: port, APIPort: 7891})

	res, err := http.Get(p.HubHealth)
	if err != nil {
		t.Fatalf("the plan's hub health URL (%s) does not reach a hub bound to --host %s: %v", p.HubHealth, host, err)
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		t.Fatalf("health probe got %d", res.StatusCode)
	}

	// And the loopback URL the plan used to hardcode does NOT answer, which is
	// what made this silent.
	loopback := strings.Replace(p.HubHealth, host, "127.0.0.1", 1)
	if _, err := http.Get(loopback); err == nil {
		t.Skip("loopback answered too (something else holds the port); the differential is unobservable here")
	}
}

func TestServePassesTrustedHostsToTheHub(t *testing.T) {
	p := buildServePlan(serveOptions{Host: "127.0.0.1", HubPort: 7895, APIPort: 7891, TrustedHosts: "node.tail1234.ts.net"})
	joined := strings.Join(p.Hub.Args, " ")
	if !strings.Contains(joined, "--trusted-host node.tail1234.ts.net") {
		t.Fatalf("--trusted-host never reaches the hub: %v", p.Hub.Args)
	}
	bare := buildServePlan(serveOptions{Host: "127.0.0.1", HubPort: 7895, APIPort: 7891})
	if strings.Contains(strings.Join(bare.Hub.Args, " "), "--trusted-host") {
		t.Fatalf("--trusted-host must not be passed when unset: %v", bare.Hub.Args)
	}
}
