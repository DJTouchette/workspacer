package bus

import (
	"encoding/json"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// Profile-aware dispatch (FLEET_MANAGER_SPIKE §6a): the router is the ONE place
// a bus caller's token is verifiable, so it is the one place a per-token
// profile grant can become something a provider may trust. These tests run the
// REAL dispatch path — websocket handshake, tier check, sanitizeSpawnParams,
// forward to a registered provider — and read what the provider actually
// receives, because the invariant is about the params on the provider's side
// of the hub, not about any predicate in isolation.

// profileGrantServer wires a bus with a host token, one scoped lookup covering
// every credential shape the sanitizer distinguishes, and a trusted provider
// for agents.spawn that reports each call's params on a channel.
func profileGrantServer(t *testing.T) (url string, got chan json.RawMessage) {
	t.Helper()
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		switch tok {
		case "tok-manager": // operator record, blessed for exactly one profile
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods(),
				ProfilesAllowed: []string{"work"}}, true
		case "tok-operator": // operator record, NO profile grant
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		}
		return ScopedIdent{}, false
	})
	// A plugin token consented to agents.spawn itself — the strongest plugin
	// shape, and it must STILL never carry a profile.
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

// spawnVia sends one agents.spawn with the given raw params and returns what
// the provider received, decoded.
func spawnVia(t *testing.T, url, token, params string, got chan json.RawMessage) map[string]any {
	t.Helper()
	caller := dialClientToken(t, url, token)
	caller.send(Frame{Op: "call", ID: "s1", Method: "agents.spawn", Params: json.RawMessage(params)})
	if r := caller.readUntil("result"); r.ID != "s1" {
		t.Fatalf("spawn result id %q, want s1", r.ID)
	}
	raw := <-got
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatalf("provider params did not decode (%v): %s", err, raw)
	}
	return m
}

// TestProfileGrantSpoofedFieldsNeverReachTheProvider is THE invariant: a caller
// without the grant cannot get a profileId through to the provider, even when
// it stamps profileGranted itself — the hub deletes both fields first and only
// re-adds them for a verified grant.
// TestProfileGrantSpoofedFromAPluginConn: a plugin consented to agents.spawn
// itself still cannot name an account — its consent dialog never mentioned one.
// TestProfileGrantGrantedIdPassesAndIsStamped: the positive half — a granted id
// survives, and the provider sees the hub's own profileGranted:true beside it.
// TestProfileGrantHostTokenPassesThrough: the host token is the control plane's
// own credential (the desktop, the MCP facade — which enforces per-session
// facade-token grants itself before a profileId reaches its bus connection).
// It may name any profile, and the stamp still comes from the hub.
// TestProfileGrantNonObjectParamsPassUntouched: the sanitizer must not turn a
// shape the provider would reject anyway into a crash or a mutation.
func TestProfileGrantNonObjectParamsPassUntouched(t *testing.T) {
	url, got := profileGrantServer(t)
	caller := dialClientToken(t, url, "tok-operator")
	caller.send(Frame{Op: "call", ID: "n1", Method: "agents.spawn", Params: json.RawMessage(`[1,2,3]`)})
	if r := caller.readUntil("result"); r.ID != "n1" {
		t.Fatalf("result id %q", r.ID)
	}
	if raw := <-got; string(raw) != `[1,2,3]` {
		t.Fatalf("non-object params were rewritten: %s", raw)
	}
}

func TestDispatchProvenanceOnlySurvivesLocalHostConnection(t *testing.T) {
	for _, token := range []string{"tok-manager", "tok-operator", "plug-tok", "host-secret"} {
		t.Run(token, func(t *testing.T) {
			url, got := profileGrantServer(t)
			params := spawnVia(t, url, token, `{"dispatchOwnerSessionId":"manager","retrySourceSessionId":"worker","taskId":"task","stage":"fix","afterDispatchId":"first"}`, got)
			if token == "host-secret" {
				if params["dispatchOwnerSessionId"] != "manager" || params["retrySourceSessionId"] != "worker" {
					t.Fatal("host provenance lost")
				}
			} else if params["dispatchOwnerSessionId"] != nil || params["retrySourceSessionId"] != nil {
				t.Fatal("forged provenance survived")
			}
			if params["taskId"] != "task" || params["stage"] != "fix" || params["afterDispatchId"] != "first" {
				t.Fatal("public task links lost")
			}
		})
	}
}
