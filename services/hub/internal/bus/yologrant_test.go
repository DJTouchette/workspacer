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

// TestYoloGrantSpoofedStampNeverReachesTheProvider is THE invariant: a caller
// without the grant cannot get `yoloGranted` through to the provider, even
// when it stamps the flag itself — the hub deletes it from every incoming call
// and only re-adds it for a verified grant. The REQUEST fields ride untouched:
// skipPermissions still reaches the provider, whose clamp then applies.
func TestYoloGrantSpoofedStampNeverReachesTheProvider(t *testing.T) {
	url, got := yoloGrantServer(t)

	cases := []struct {
		name, token, params string
	}{
		{"operator record without grant, honest request", "tok-operator", `{"cwd":"/tmp","skipPermissions":true}`},
		{"operator record without grant, self-stamped", "tok-operator", `{"cwd":"/tmp","skipPermissions":true,"yoloGranted":true}`},
		{"profile grant does not imply the full-access grant", "tok-profiles", `{"cwd":"/tmp","skipPermissions":true,"yoloGranted":true}`},
		{"plugin conn that consented to agents.spawn itself", "plug-tok", `{"cwd":"/tmp","skipPermissions":true,"yoloGranted":true}`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			m := spawnVia(t, url, c.token, c.params, got)
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

// TestYoloGrantGrantedTokenIsStamped: the positive half — a granted caller's
// spawn arrives with the hub's own yoloGranted:true, whether or not the caller
// asked for a bypass (the stamp is about the CALLER; the provider still reads
// the request's skipPermissions to decide what to do with it).
func TestYoloGrantGrantedTokenIsStamped(t *testing.T) {
	url, got := yoloGrantServer(t)
	m := spawnVia(t, url, "tok-full", `{"cwd":"/tmp","skipPermissions":true}`, got)
	if m["yoloGranted"] != true {
		t.Fatalf("hub did not stamp yoloGranted for a granted caller: %v", m)
	}
	if m["skipPermissions"] != true {
		t.Fatalf("the granted request's own skipPermissions must ride through: %v", m)
	}
	// A caller-supplied yoloGranted on a GRANTED spawn is also fine — deleted
	// and re-stamped, indistinguishable from the honest call.
	m = spawnVia(t, url, "tok-full", `{"cwd":"/tmp","yoloGranted":true}`, got)
	if m["yoloGranted"] != true {
		t.Fatalf("granted spawn with a redundant self-stamp: %v", m)
	}
	if _, has := m["skipPermissions"]; has {
		t.Fatalf("the stamp must not invent a bypass request the caller never made: %v", m)
	}
}

// TestYoloGrantHostTokenIsStamped: the host token is the control plane's own
// credential; its spawns carry the stamp, and the stamp still comes from the
// hub (any self-stamp was deleted first).
func TestYoloGrantHostTokenIsStamped(t *testing.T) {
	url, got := yoloGrantServer(t)
	m := spawnVia(t, url, "host-secret", `{"cwd":"/tmp","skipPermissions":true}`, got)
	if m["yoloGranted"] != true || m["skipPermissions"] != true {
		t.Fatalf("host-token spawn should keep its request and gain the stamp: %v", m)
	}
}
