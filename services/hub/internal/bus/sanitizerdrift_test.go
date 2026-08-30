package bus

import (
	"encoding/json"
	"testing"
)

// TestFederatedCallSanitizesThroughTheSameTableAsLocalCall is the structural
// guard for the class of bug fixed in
// TestReportProgressCallerSessionIsStrippedBeforeTheFederatedHop:
// router.call and router.federatedCall used to hand-repeat their own `if
// method == X` sanitizer lists, and the federated one forgot a method the
// local one had. That is now impossible by construction — both dispatch
// points call [sanitizeCallParams] against the single [methodSanitizers] map
// — but a structural claim like "impossible by construction" is worth nothing
// until something has actually watched it hold. This test registers a THIRD,
// throwaway sanitizer directly into methodSanitizers (nothing bespoke to
// either call() or federatedCall()) and proves both dispatch paths pick it up
// with zero code changes on either side.
func TestFederatedCallSanitizesThroughTheSameTableAsLocalCall(t *testing.T) {
	const thirdMethod = "test.thirdSanitizer"
	stripped := false
	methodSanitizers[thirdMethod] = func(rt *router, caller *conn, raw json.RawMessage) json.RawMessage {
		stripped = true
		var m map[string]json.RawMessage
		if err := json.Unmarshal(raw, &m); err != nil || m == nil {
			return raw
		}
		delete(m, "secret")
		out, err := json.Marshal(m)
		if err != nil {
			return json.RawMessage("{}")
		}
		return out
	}
	t.Cleanup(func() { delete(methodSanitizers, thirdMethod) })

	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		if tok == "tok-x" {
			return ScopedIdent{Scope: "test", Methods: []string{thirdMethod}}, true
		}
		return ScopedIdent{}, false
	})

	// Local path: a trusted provider answers the bare method, a scoped caller
	// invokes it directly.
	got := make(chan json.RawMessage, 4)
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{thirdMethod}})
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

	caller := dialClientToken(t, url, "tok-x")
	caller.send(Frame{Op: "call", ID: "l1", Method: thirdMethod,
		Params: json.RawMessage(`{"secret":"leak","note":"keep"}`)})
	if r := caller.readUntil("result"); r.ID != "l1" {
		t.Fatalf("local result id %q, want l1", r.ID)
	}
	var localParams map[string]any
	if err := json.Unmarshal(<-got, &localParams); err != nil {
		t.Fatalf("provider params did not decode: %v", err)
	}
	if !stripped {
		t.Fatal("the throwaway sanitizer never ran on the local call path")
	}
	if _, has := localParams["secret"]; has {
		t.Errorf("local call: throwaway sanitizer did not strip secret: %v", localParams)
	}
	if localParams["note"] != "keep" {
		t.Errorf("local call: sanitizer damaged unrelated params: %v", localParams)
	}

	// Federated path: same method, same tier, no provider needed — the fake
	// federation forwarder records what actually crossed the hop.
	stripped = false
	fed := &fakeFed{got: make(chan json.RawMessage, 4)}
	srv.SetFederation(fed)
	caller.send(Frame{Op: "call", ID: "f1", Method: "hub:work/" + thirdMethod,
		Params: json.RawMessage(`{"secret":"leak","note":"keep"}`)})
	if r := caller.readUntil("result"); r.ID != "f1" {
		t.Fatalf("federated result id %q, want f1", r.ID)
	}
	var fedParams map[string]any
	if err := json.Unmarshal(<-fed.got, &fedParams); err != nil {
		t.Fatalf("forwarded params did not decode: %v", err)
	}
	if !stripped {
		t.Fatal("the throwaway sanitizer never ran on the federated call path — the two dispatch points have drifted apart")
	}
	if _, has := fedParams["secret"]; has {
		t.Errorf("federated call: throwaway sanitizer did not strip secret before the hop: %v", fedParams)
	}
	if fedParams["note"] != "keep" {
		t.Errorf("federated call: sanitizer damaged unrelated params: %v", fedParams)
	}
}
