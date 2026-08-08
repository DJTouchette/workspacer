package push

import (
	"encoding/json"
	"net"
	"strings"
	"testing"
)

// PROVEN. push.subscribe (TRIAGE tier — the /m phone) made the hub POST to any
// URL the caller named. The endpoint is stored by one call and used by a
// different subsystem (Watch -> onSnapshot -> sendOne) to issue a request from
// the HOST's network position, for a tier that holds no fetch, no exec, no fs
// and no config capability at all — and the same tier holds agents.sendMessage
// and claude.approve, so it can pull the trigger on demand.
func TestPushSubscribeRefusesNonPushEndpoints(t *testing.T) {
	m, _ := newTestManager(t)
	who := Subscriber{TokenID: "phone-token", Scope: "triage"}

	refused := []string{
		"http://127.0.0.1:7895/plugins/install",
		"http://169.254.169.254/latest/meta-data/",
		"https://169.254.169.254/latest/meta-data/",
		"https://10.0.0.5/internal/admin",
		"https://192.168.1.1/",
		"https://[::1]/x",
		"https://[::ffff:127.0.0.1]/x",
		"https://[fd00::1]/x",
		"file:///etc/passwd",
		"ftp://example.invalid/x",
		"not a url at all",
	}
	for _, ep := range refused {
		params, _ := json.Marshal(map[string]any{
			"endpoint": ep,
			"keys":     map[string]string{"p256dh": "k", "auth": "a"},
		})
		if _, err := m.RPCSubscribeAs(who, params); err == nil {
			t.Errorf("push.subscribe accepted %q — the hub would POST there with a VAPID header from its own network position", ep)
		}
	}
	if n := len(m.subs); n != 0 {
		t.Fatalf("%d refused endpoints were stored anyway", n)
	}

	// FLOOR: a real Web Push endpoint must still work, or the fix is a deletion
	// of the feature. This is the shape every browser produces.
	for _, ep := range []string{
		"https://fcm.googleapis.com/fcm/send/abc123",
		"https://updates.push.services.mozilla.com/wpush/v2/gAAAA",
		"https://push.example.com:8443/p/xyz",
	} {
		params, _ := json.Marshal(map[string]any{
			"endpoint": ep,
			"keys":     map[string]string{"p256dh": "k", "auth": "a"},
		})
		if _, err := m.RPCSubscribeAs(who, params); err != nil {
			t.Errorf("push.subscribe refused a legitimate endpoint %q: %v", ep, err)
		}
	}
	if n := len(m.subs); n != 3 {
		t.Fatalf("stored %d legitimate subscriptions, want 3", n)
	}
}

// EVERY CLAUSE OF isPublicIP, one address each, chosen so that NO OTHER clause
// catches it. Deleting any single predicate therefore fails here by name.
//
// The previous shape of this function claimed two more cases than it had: an
// IPv4-mapped-IPv6 recursion that net.IP.Equal made unreachable (v4.Equal(ip) is
// true for ::ffff:127.0.0.1 vs 127.0.0.1, so the `!v4.Equal(ip)` guard never
// fired) and an fc00::/7 branch Go's own IsPrivate had already answered. Both
// were deleted; the BEHAVIOUR they claimed is asserted below instead, which is
// the difference between a comment and a guard.
func TestIsPublicIPRefusesEveryNonRoutableFamily(t *testing.T) {
	// sole witness for -> address. If a case ever starts being caught by a
	// second clause it stops proving anything, so the loop below also checks
	// that the address is refused for the reason claimed.
	cases := []struct {
		clause, ip, why string
	}{
		{"IsLoopback", "127.0.0.1", "the hub's own HTTP surface"},
		{"IsLoopback", "::1", "the same, in v6"},
		{"IsLoopback", "::ffff:127.0.0.1", "the v4-mapped spelling of loopback — the notation the deleted recursion claimed to cover"},
		{"IsPrivate", "10.0.0.5", "RFC1918"},
		{"IsPrivate", "192.168.1.1", "RFC1918"},
		{"IsPrivate", "172.16.0.1", "RFC1918, the range most often forgotten"},
		{"IsPrivate", "::ffff:10.0.0.5", "v4-mapped RFC1918"},
		{"IsPrivate", "fd00::1", "IPv6 unique-local — Go's IsPrivate covers fc00::/7"},
		{"IsPrivate", "fc00::1", "the other half of fc00::/7, which the deleted branch claimed"},
		{"IsUnspecified", "0.0.0.0", "connects to this host"},
		{"IsUnspecified", "::", "the same, in v6"},
		{"IsLinkLocalUnicast", "169.254.169.254", "the cloud metadata address — the whole reason this is not IsLoopback||IsPrivate"},
		{"IsLinkLocalUnicast", "::ffff:169.254.169.254", "metadata, v4-mapped"},
		{"IsLinkLocalUnicast", "fe80::1", "IPv6 link-local"},
		{"IsMulticast", "224.0.0.1", "all-hosts, v4"},
		{"IsMulticast", "ff02::1", "link-local scope"},
		{"IsMulticast", "ff01::1", "interface-local scope"},
		{"IsMulticast", "ff0e::1", "GLOBAL scope — caught by IsMulticast and by nothing else, which is why the two narrower multicast predicates were redundant"},
	}
	for _, c := range cases {
		ip := net.ParseIP(c.ip)
		if ip == nil {
			t.Fatalf("test bug: %q does not parse", c.ip)
		}
		if isPublicIP(ip) {
			t.Errorf("isPublicIP(%s) = true — %s is %s, and the hub is the only thing that can reach it. The %s clause is gone.",
				c.ip, c.ip, c.why, c.clause)
		}
	}

	// FLOOR: real push services must still resolve as public, or the guard is a
	// blanket refusal of every literal IP and the fix is a deletion of the
	// feature.
	for _, ip := range []string{"8.8.8.8", "142.250.185.106", "2607:f8b0:4004::200e", "2001:4860:4860::8888"} {
		if !isPublicIP(net.ParseIP(ip)) {
			t.Errorf("isPublicIP(%s) = false — a routable address was refused", ip)
		}
	}
}

// The mapped-notation claim, at the level a caller actually meets it: the
// bypass is a STRING in an RPC parameter, not a net.IP.
func TestMappedNotationDoesNotBypassTheEndpointRule(t *testing.T) {
	for _, ep := range []string{
		"https://[::ffff:127.0.0.1]/x",
		"https://[::ffff:10.0.0.5]/x",
		"https://[::ffff:169.254.169.254]/latest/meta-data/",
		"https://[::ffff:192.168.0.1]/x",
	} {
		if err := validatePushEndpoint(ep); err == nil {
			t.Errorf("validatePushEndpoint(%q) accepted it — the same address in another notation is the same address, and the hub would POST there", ep)
		}
	}
}

// The failure has to say what is wrong, or the phone's registration silently
// stops working and nobody can tell why.
func TestPushEndpointRefusalNamesTheReason(t *testing.T) {
	err := validatePushEndpoint("http://127.0.0.1:9/x")
	if err == nil {
		t.Fatal("http loopback accepted")
	}
	if !strings.Contains(err.Error(), "https") {
		t.Errorf("error %q does not say what the endpoint must be", err)
	}
}
