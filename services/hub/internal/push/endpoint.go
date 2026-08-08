package push

import (
	"fmt"
	"net"
	"net/url"
	"strings"
)

// THE ENDPOINT IS A NETWORK SINK, not a string. push.subscribe made the hub POST
// to any URL the caller named, on a trigger the same tier can pull at will.
//
// The tier is TRIAGE — the /m phone tier, which holds push.subscribe and holds
// no fetch, no exec, no fs and no config capability at all. RPCSubscribeAs
// validated only that endpoint, keys.p256dh and keys.auth were non-empty, stored
// the row, and Watch/sendOne later handed the endpoint to
// webpush.SendNotificationWithContext, which issues POST <endpoint> with a VAPID
// Authorization header and an aes128gcm body — from the HOST's network position:
// Tailscale-reachable, loopback-reachable, cloud-metadata-reachable. The same
// tier holds agents.sendMessage and claude.approve/answer, so it can drive an
// agent into and out of the blocked state to fire the request on demand and
// repeatedly.
//
// Proven with the real Manager and a real trigger:
//
//	push.subscribe(endpoint="http://127.0.0.1:45007/internal/admin/action?x=1") ACCEPTED
//	THE HUB MADE THE REQUEST: POST /internal/admin/action headers=[TTL Authorization Content-Encoding]
//
// and the validation surface accepted http://169.254.169.254/latest/meta-data/,
// file:///etc/passwd, ftp://…, and "not a url at all".
//
// capspec's excuse for the capability reasons entirely about what the ENDPOINT
// learns — "the payload is encrypted to the subscription's own keys, so the
// endpoint learns nothing it did not supply" — and never about what the HOST is
// made to do. That is the WIDEN-THEN-USE shape: one call writes a row, a
// different subsystem consults that row to act.
//
// The rule below is what a Web Push endpoint actually is: an https URL at a push
// service (Google/Mozilla/Apple/Microsoft/self-hosted), on the public internet.
// Nothing legitimate about it is http, and nothing legitimate about it is a
// private, loopback, link-local or unique-local address. Refusing those costs
// real deployments nothing and takes the host's network position off the table.

// validatePushEndpoint returns an error when the endpoint is not something a
// browser's PushManager could plausibly have produced.
//
// Hostnames that are not literal IPs are accepted: resolving them here would be
// a DNS lookup on the caller's string (and a TOCTOU besides — the name can be
// re-pointed between the check and the send). The scheme requirement is what
// bounds that case: an https endpoint reached over TLS is the shape the whole
// protocol has, and an internal service impersonating a push endpoint over TLS
// is a materially different attack from "POST to 127.0.0.1".
func validatePushEndpoint(endpoint string) error {
	u, err := url.Parse(endpoint)
	if err != nil {
		return fmt.Errorf("push.subscribe: endpoint is not a URL: %w", err)
	}
	if !strings.EqualFold(u.Scheme, "https") {
		return fmt.Errorf("push.subscribe: endpoint scheme %q is not https — a Web Push endpoint is always https, and any other scheme makes the hub issue a request of the caller's choosing from the host's own network position", u.Scheme)
	}
	host := u.Hostname()
	if host == "" {
		return fmt.Errorf("push.subscribe: endpoint %q has no host", endpoint)
	}
	if ip := net.ParseIP(host); ip != nil && !isPublicIP(ip) {
		return fmt.Errorf("push.subscribe: endpoint host %q is a loopback, private, link-local or otherwise non-public address — no push service lives there, and the hub is the only thing that can reach it", host)
	}
	return nil
}

// isPublicIP reports whether ip is routable on the public internet.
//
// Five predicates, each carrying its own case and none of them redundant —
// TestIsPublicIPRefusesEveryNonRoutableFamily gives each one an address that
// ONLY it catches, so deleting any single clause fails by name:
//
//   - IsLoopback     127.0.0.1, ::1 — the hub's own admin surface
//   - IsPrivate      10/8, 172.16/12, 192.168/16 AND IPv6 unique-local fc00::/7.
//     Go's IsPrivate already implements the ULA range (ip[0]&0xfe == 0xfc), so
//     there is no separate fc00 clause to write; fd00::1 and fc00::1 land here.
//   - IsUnspecified  0.0.0.0 / :: — routes to "this host" on connect
//   - IsLinkLocal…   169.254.169.254, the cloud metadata address, and fe80::/10.
//     This is the reason the function is not just IsLoopback||IsPrivate.
//   - IsMulticast    224.0.0.0/4 and ff00::/8, including the link-local and
//     interface-local scopes: IsLinkLocalMulticast and
//     IsInterfaceLocalMulticast are strict SUBSETS of it, so naming them too
//     added two clauses no address could reach on its own.
//
// There is deliberately no IPv4-mapped-IPv6 recursion. ::ffff:127.0.0.1 is
// caught by IsLoopback directly — every predicate above resolves To4() first,
// and net.IP.Equal treats the two notations as one address — so the branch that
// re-judged the mapped form was unreachable, and reachability is what the test
// asserts (the mapped spellings are refused, by these clauses, with no
// recursion in the way).
// There IS a sixth clause, and it is the IPv4-COMPATIBLE form ::a.b.c.d —
// distinct from the v4-mapped ::ffff:a.b.c.d above. Go resolves To4() for the
// mapped spelling and not for the compatible one, so ::127.0.0.1 parses to
// ::7f00:1, answers false to every predicate here, and was judged PUBLIC:
// validatePushEndpoint accepted https://[::127.0.0.1]/… and https://[::169.254.169.254]/….
//
// Measured honestly, that is a classification error and not a live escape: a
// dial to [::127.0.0.1] TIMES OUT, because the kernel does not route the
// deprecated IPv4-compatible form to the v4 loopback (only the mapped form
// routes, and that one is already refused). It is fixed anyway, on the rule this
// function is written to: refusing it costs no real deployment anything, and a
// predicate that says "public" about an address nobody can reach is a wrong
// answer waiting for a platform that resolves it differently.
func isPublicIP(ip net.IP) bool {
	return !(ip.IsLoopback() || ip.IsPrivate() || ip.IsUnspecified() ||
		ip.IsLinkLocalUnicast() || ip.IsMulticast() || isIPv4Compatible(ip))
}

// isIPv4Compatible reports whether ip is the deprecated IPv4-compatible IPv6
// form: 96 zero bits followed by an IPv4 address (RFC 4291 §2.5.5.1, deprecated
// by RFC 4291 itself). :: and ::1 also match the bit pattern and are caught by
// IsUnspecified / IsLoopback; naming them here too would cost nothing and is not
// the point of the clause.
func isIPv4Compatible(ip net.IP) bool {
	ip16 := ip.To16()
	if ip16 == nil || ip.To4() != nil {
		// Not IPv6 at all, or the v4-mapped spelling, which To4() resolves and
		// the clauses above already judge on its embedded v4 address.
		return false
	}
	for _, b := range ip16[:12] {
		if b != 0 {
			return false
		}
	}
	return true
}
