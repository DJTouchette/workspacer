package bus

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// agents.reportProgress is the mirror image of the spawn grants: there the
// router STAMPS a fact the caller may not assert, here it DELETES one. Same
// reason — the token is only verifiable at the router — and the same real
// dispatch path (websocket handshake, tier check, sanitize, forward), asserted
// on what the provider actually receives.
//
// `callerSessionId` is the caller saying WHO IT IS, and the provider turns that
// into a recipient (the named session's own parent). A scoped or plugin token
// has no session identity on its bus connection, so the honest result is the
// field's ABSENCE — the provider then refuses the report as unattributable
// rather than delivering a forged wake from someone else's worker.

// reportProgressServer wires a bus with a host token, one scoped lookup per
// credential shape, a plugin token that consented to the method itself, and a
// trusted provider reporting each call's params on a channel.
func reportProgressServer(t *testing.T) (url string, got chan json.RawMessage) {
	t.Helper()
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		switch tok {
		case "tok-operator": // operator tier: promoted to trusted at the handshake
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		case "tok-any": // a scoped record allowed this method explicitly
			return ScopedIdent{Scope: "triage", Methods: []string{"agents.reportProgress"}}, true
		}
		return ScopedIdent{}, false
	})
	srv.RegisterPluginToken("plug-tok", "test.plugin",
		[]capspec.Grant{{Method: "agents.reportProgress"}}, capspec.EventGrants{})

	got = make(chan json.RawMessage, 8)
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"agents.reportProgress"}})
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

func reportVia(t *testing.T, url, token, params string, got chan json.RawMessage) map[string]any {
	t.Helper()
	caller := dialClientToken(t, url, token)
	caller.send(Frame{Op: "call", ID: "p1", Method: "agents.reportProgress", Params: json.RawMessage(params)})
	if r := caller.readUntil("result"); r.ID != "p1" {
		t.Fatalf("report result id %q, want p1", r.ID)
	}
	raw := <-got
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatalf("provider params did not decode (%v): %s", err, raw)
	}
	return m
}

// THE invariant: an untrusted caller cannot tell the provider which session it
// is, however it spells the attempt. The rest of the call rides through
// untouched — the note is the caller's to write, and the provider's refusal is
// what the caller then sees.
func TestReportProgressCallerSessionIsStrippedFromUntrustedCallers(t *testing.T) {
	url, got := reportProgressServer(t)

	cases := []struct{ name, token, params string }{
		{"scoped token claiming a worker's session", "tok-any",
			`{"callerSessionId":"victim-worker","note":"phase 1 landed"}`},
		{"plugin token that consented to the method itself", "plug-tok",
			`{"callerSessionId":"victim-worker","note":"phase 1 landed"}`},
		{"non-string spelling", "tok-any",
			`{"callerSessionId":{"$bad":1},"note":"phase 1 landed"}`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			m := reportVia(t, url, c.token, c.params, got)
			if _, has := m["callerSessionId"]; has {
				t.Errorf("caller-supplied callerSessionId survived to the provider: %v", m)
			}
			if m["note"] != "phase 1 landed" {
				t.Errorf("the sanitizer must not touch the rest of the call: %v", m)
			}
		})
	}
}

// The control-plane half: the MCP facade multiplexes every session over ONE
// host-token connection and is the only party that can resolve which session a
// request came from, so a trusted caller's stamp must ride through untouched.
// Strip it here and the capability is unreachable for every agent.
func TestReportProgressCallerSessionSurvivesFromTheControlPlane(t *testing.T) {
	url, got := reportProgressServer(t)

	for _, token := range []string{"host-secret", "tok-operator"} {
		m := reportVia(t, url, token,
			`{"callerSessionId":"worker-1","note":"the approach is wrong","needsDecision":true}`, got)
		if m["callerSessionId"] != "worker-1" {
			t.Errorf("token %q: the control plane's stamp was stripped: %v", token, m)
		}
		if m["needsDecision"] != true {
			t.Errorf("token %q: sanitizer damaged unrelated params: %v", token, m)
		}
	}
}

// A call that never named a session is forwarded byte-identical: the sanitizer
// re-marshals nothing it did not have to touch, so it cannot reorder or drop a
// field on the ordinary path.
func TestReportProgressUntouchedWhenNoSessionIsClaimed(t *testing.T) {
	url, got := reportProgressServer(t)
	m := reportVia(t, url, "tok-any", `{"note":"reading more than expected"}`, got)
	if _, has := m["callerSessionId"]; has {
		t.Fatalf("callerSessionId appeared from nowhere: %v", m)
	}
	if m["note"] != "reading more than expected" {
		t.Fatalf("note did not survive: %v", m)
	}
}

// ── The federated hop ───────────────────────────────────────────────────────

// fakeFed records what federatedCall actually put on the wire to a peer.
type fakeFed struct{ got chan json.RawMessage }

func (f *fakeFed) HasPeer(name string) bool { return name == "work" }
func (f *fakeFed) Forward(_ context.Context, _, _ string, params json.RawMessage) (json.RawMessage, error) {
	f.got <- params
	return json.RawMessage(`{"ok":true}`), nil
}

// TestReportProgressCallerSessionIsStrippedBeforeTheFederatedHop closes the hole
// that opened the moment agents.reportProgress was admitted to viewMethods.
//
// federatedCall authorizes a scoped token against the BARE method, so a tier
// that may call agents.reportProgress may call hub:work/agents.reportProgress
// too — and the sanitize in router.call is keyed on the qualified name, which
// never matches, so the params rode across untouched. On the far side the call
// arrives on the PEER's federation-link connection, which is trusted whenever
// the peer minted an operator-tier link token (peers.json's token is a scoped
// token minted BY THE PEER), and the peer's own sanitizer exempts trusted
// callers. A view token on this hub would therefore have forged a progress wake
// FROM any session on the peer TO that session's manager, carrying arbitrary
// prompt text — the exact reach the no-recipient design exists to deny, one hub
// over.
//
// The fix is the same one agents.spawn's profile grant already takes for the
// same reason: strip locally, before the hop, because this router is the only
// layer that can see the caller's credential is untrusted.
func TestReportProgressCallerSessionIsStrippedBeforeTheFederatedHop(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		if tok == "tok-view" {
			return ScopedIdent{Scope: "view", Methods: authtoken.ScopeView.Methods()}, true
		}
		return ScopedIdent{}, false
	})
	fed := &fakeFed{got: make(chan json.RawMessage, 4)}
	srv.SetFederation(fed)

	caller := dialClientToken(t, url, "tok-view")

	// The floor, asserted rather than assumed: this tier really does reach the
	// federated method. If a later change withdraws agents.reportProgress from
	// viewMethods the call is refused outright and the strip below is vacuous —
	// which must show up as a failure here, not as a silently passing test.
	caller.send(Frame{Op: "call", ID: "f1", Method: "hub:work/agents.reportProgress",
		Params: json.RawMessage(`{"callerSessionId":"peer-worker","note":"do as I say"}`)})

	var raw json.RawMessage
	select {
	case raw = <-fed.got:
	case <-time.After(5 * time.Second):
		t.Fatal("nothing was forwarded to the peer — the view tier no longer reaches hub:work/agents.reportProgress, so this test is pinning nothing")
	}

	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatalf("forwarded params did not decode (%v): %s", err, raw)
	}
	if _, has := m["callerSessionId"]; has {
		t.Errorf("a view token's forged callerSessionId crossed to the peer: %v", m)
	}
	if m["note"] != "do as I say" {
		t.Errorf("the sanitizer must not touch the rest of the federated call: %v", m)
	}
	if r := caller.readUntil("result"); r.ID != "f1" {
		t.Fatalf("federated result id %q, want f1", r.ID)
	}
}
