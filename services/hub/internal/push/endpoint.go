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

// isPublicIP reports whether ip is routable on the public internet. Link-local
// covers 169.254.169.254, the cloud metadata address, which is the reason this
// function is not just IsLoopback||IsPrivate.
func isPublicIP(ip net.IP) bool {
	if ip.IsLoopback() || ip.IsPrivate() || ip.IsUnspecified() ||
		ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() ||
		ip.IsInterfaceLocalMulticast() || ip.IsMulticast() {
		return false
	}
	// IPv4-mapped IPv6 (::ffff:127.0.0.1) must be judged as the v4 address it
	// carries, or every check above is one notation away from being bypassed.
	if v4 := ip.To4(); v4 != nil && !v4.Equal(ip) {
		return isPublicIP(v4)
	}
	// Unique-local IPv6 (fc00::/7) has no Go stdlib predicate.
	if v6 := ip.To16(); v6 != nil && ip.To4() == nil && v6[0]&0xfe == 0xfc {
		return false
	}
	return true
}
