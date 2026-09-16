package bus

import (
	"encoding/json"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// Full-access dispatch (the Fleet Manager's --dangerously-skip-permissions
// grant): like the profile grant, the router is the ONE place a bus caller's
// token is verifiable, so it is the one place authtoken's yoloAllowed can
// become something a provider may trust — the hub-only `yoloGranted` stamp.
// Unlike the profile grant, the sanitizer never touches the REQUEST itself:
// `skipPermissions` rides through either way, and an unstamped request keeps
// the provider's clamp. Same real-dispatch method as profilegrant_test.go —
// websocket handshake, tier check, sanitizeSpawnParams, forward — asserted on
// what the provider actually receives.

// yoloGrantServer wires a bus with a host token, one scoped lookup covering
// every credential shape mayBypassPermissions distinguishes, and a trusted
// provider for agents.spawn that reports each call's params on a channel.
func yoloGrantServer(t *testing.T) (url string, got chan json.RawMessage) {
	t.Helper()
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		switch tok {
		case "tok-full": // operator record blessed with the full-access grant
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods(),
				YoloAllowed: true}, true
		case "tok-operator": // operator record, NO full-access grant
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		case "tok-profiles": // profile grant is NOT the full-access grant
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods(),
				ProfilesAllowed: []string{"work"}}, true
		case "tok-peer": // what a peer's peers.json usually holds: a plain operator token
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		case "tok-peer-full": // …and the same link token minted WITH the grant
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods(),
				YoloAllowed: true}, true
		}
		return ScopedIdent{}, false
	})
	// A plugin token consented to agents.spawn itself — the strongest plugin
	// shape, and it must STILL never carry the stamp.
	srv.RegisterPluginToken("plug-tok", "test.plugin",
		[]capspec.Grant{{Method: "agents.spawn"}}, capspec.EventGrants{})

	got = make(chan json.RawMessage, 8)
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"agents.spawn"}})
	provider.readUntil("registered")
	go func() {
		for {
			f, ok := provider.tryRead("call")
			if !ok {
				return
			}
			got <- f.Params
			provider.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
		}
	}()
	return url, got
}

// spawnViaPeerLink is spawnVia over a connection that declares itself a
// FEDERATION LINK, the way internal/federation dials (bus.PeerLinkParam). The
// marker rides in the query string beside the token because that is where the
// real dialer puts it — dialClientToken appends the token verbatim, so the
// suffix lands as a sibling param and `presentedToken` still reads the token.
func spawnViaPeerLink(t *testing.T, url, token, params string, got chan json.RawMessage) map[string]any {
	t.Helper()
	return spawnVia(t, url, token+"&"+PeerLinkParam+"=1", params, got)
}

// TestYoloGrantSpoofedStampNeverReachesTheProvider is THE invariant: a caller
// without the grant cannot get `yoloGranted` through to the provider, even
// when it stamps the flag itself — the hub deletes it from every incoming call
// and only re-adds it for a verified grant. The REQUEST fields ride untouched:
// skipPermissions still reaches the provider, whose clamp then applies.
//
// The population changed on 2026-08-26 and the invariant did not. An
// operator-tier token is now trusted with the bypass (ScopeOperator is
// documented as "everything — equivalent to the host remote-token", and
// withholding it was the silent downgrade this whole change exists to end), so
// the shapes that must still be refused are the two that are NOT the user:
// third-party plugin code, and another hub's link — which reaches this router
// holding whatever credential peers.json happened to carry.
func TestYoloGrantSpoofedStampNeverReachesTheProvider(t *testing.T) {
	url, got := yoloGrantServer(t)

	cases := []struct {
		name, token, params string
		peerLink            bool
	}{
		{name: "plugin conn that consented to agents.spawn itself", token: "plug-tok",
			params: `{"cwd":"/tmp","skipPermissions":true,"yoloGranted":true}`},
		{name: "federation link on a plain operator token", token: "tok-peer",
			params: `{"cwd":"/tmp","skipPermissions":true}`, peerLink: true},
		{name: "federation link self-stamping the grant", token: "tok-peer",
			params: `{"cwd":"/tmp","skipPermissions":true,"yoloGranted":true}`, peerLink: true},
		// The one that decides the federation clause: peers.json routinely holds
		// the far hub's HOST token, and host authority over the LINK must not
		// become host authority over every spawn the peer forwards.
		{name: "federation link holding the HOST token", token: "host-secret",
			params: `{"cwd":"/tmp","skipPermissions":true,"yoloGranted":true}`, peerLink: true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			var m map[string]any
			if c.peerLink {
				m = spawnViaPeerLink(t, url, c.token, c.params, got)
			} else {
				m = spawnVia(t, url, c.token, c.params, got)
			}
			if _, has := m["yoloGranted"]; has {
				t.Errorf("caller-supplied yoloGranted survived to the provider: %v", m)
			}
			if m["skipPermissions"] != true {
				t.Errorf("the sanitizer must not touch the request itself — skipPermissions was rewritten: %v", m)
			}
			if m["cwd"] != "/tmp" {
				t.Errorf("sanitizer damaged unrelated params: %v", m)
			}
		})
	}
}

// THE PRODUCT RULE, 2026-08-26: "we should allow full access. I want this to
// feel like my machine is local and this is why we have tokens." An
// operator-tier token is host-equivalent by definition (authtoken.ScopeOperator:
// "everything — equivalent to the host remote-token"), so a full-access spawn
// from the phone must actually come up full-access. Before this it was clamped
// unless someone had separately minted yoloAllowed, and the only record of the
// downgrade was a log line on the host.
// The federation grant has to be EXPLICIT — and it has to be reachable, or the
// clause above is a wall with no door. `workspacer token create --scope operator
// --full-access` mints exactly this record; putting THAT token in the peer's
// peers.json entry is what lets the peer dispatch full-access work here.
// The federation marker is self-asserted, so the only thing that must be true of
// it is that lying can never GAIN anything. An ordinary client that claims to be
// a peer link gets less, not more.
// TestYoloGrantGrantedTokenIsStamped: the positive half — a granted caller's
// spawn arrives with the hub's own yoloGranted:true, whether or not the caller
// asked for a bypass (the stamp is about the CALLER; the provider still reads
// the request's skipPermissions to decide what to do with it).
// TestYoloGrantHostTokenIsStamped: the host token is the control plane's own
// credential; its spawns carry the stamp, and the stamp still comes from the
// hub (any self-stamp was deleted first).
// NO SILENT DOWNGRADES, router half. The sanitizer is the only party that knows
// it removed `profileId` — the provider just sees a spawn without one — so it
// stamps `escalationScrubbed` for the provider to fold into its answer. Like
// the two grant stamps it is hub-only: an incoming copy is deleted first, so a
// caller can neither forge a complaint nor suppress a real one.
