package bus

import (
	"encoding/json"
	"strings"
	"testing"
)

// THE CRITICAL ONE. A SINGLE case-variant authority key must not reach a
// provider, on any path, for any field.
//
// The old guard (cmd/brain's rejectCaseVariantKeys) only fired when BOTH
// spellings were present, so `{"YoloGranted":true}` on its own sailed through a
// sanitizer that deletes `yoloGranted` and bound to spawnParams.YoloGranted
// anyway — encoding/json matches struct tags case-insensitively. Every test here
// runs the WHOLE path (handshake, tier check, sanitizeSpawnParams, forward) and
// asserts nothing was forwarded at all.

// spawnRefused sends one agents.spawn and asserts the caller got an error rather
// than a spawn. The provider channel is checked EMPTY afterwards: an error
// returned to the caller while the params still reached a provider would be the
// same bypass with better manners.
func spawnRefused(t *testing.T, url, token, params string, got chan json.RawMessage) string {
	t.Helper()
	caller := dialClientToken(t, url, token)
	caller.send(Frame{Op: "call", ID: "s1", Method: "agents.spawn", Params: json.RawMessage(params)})
	f := caller.readUntil("error")
	if f.ID != "s1" {
		t.Fatalf("error frame id %q, want s1", f.ID)
	}
	select {
	case raw := <-got:
		t.Fatalf("the spawn was refused to the caller and STILL reached the provider: %s", raw)
	default:
	}
	return f.Error
}

// TestEveryAuthorityFieldsCaseVariantIsRefused enumerates the whole spawn
// surface — not just the four fields the review named — because the bypass is a
// property of the MATCHING, not of any one field.
func TestEveryAuthorityFieldsCaseVariantIsRefused(t *testing.T) {
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "balanced", MaxToolScope: "view"}
	})

	// One case variant per canonical key, generated from the canonical list
	// itself so a key added to spawnkeys.go is covered without editing this test.
	for _, canon := range SpawnParamKeys() {
		variant := strings.ToUpper(canon[:1]) + canon[1:]
		if variant == canon {
			t.Fatalf("canonical key %q is not lower-camel; the variant generator needs revisiting", canon)
		}
		t.Run(variant, func(t *testing.T) {
			params := `{"cwd":"/tmp",` + jsonKey(variant) + `:true}`
			msg := spawnRefused(t, url, "tok-operator", params, got)
			if !strings.Contains(msg, variant) || !strings.Contains(msg, canon) {
				t.Errorf("the refusal must name both the spelling sent and the one allowed; got %q", msg)
			}
		})
	}
}

// jsonKey quotes a key for embedding in a JSON literal.
func jsonKey(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}

// TestTheReviewsThreeConcreteBypassShapesAreRefused is the report's own payloads,
// verbatim, with their real values rather than the generated `true`.
func TestTheReviewsThreeConcreteBypassShapesAreRefused(t *testing.T) {
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		// A ceiling that would clamp everything, so a bypass would be visible as
		// an UNCLAMPED param on the provider side if one got through.
		v := SpawnCeilingVerdict{Key: "default", MaxCapability: "balanced", MaxToolScope: "view"}
		if req.Capability != "" && req.Capability != "balanced" {
			v.CapabilityRefused, v.Capability = true, "balanced"
		}
		if req.ToolScope != "" && req.ToolScope != "view" {
			v.ToolScopeRefused, v.ToolScope = true, "view"
		}
		return v
	})

	for _, tc := range []struct{ name, params string }{
		{"the full-access stamp", `{"cwd":"/capped","YoloGranted":true,"skipPermissions":true}`},
		{"the capability ceiling", `{"cwd":"/capped","Capability":"frontier_plus","Model":"fable","Effort":"high"}`},
		{"the legacy facade flag", `{"cwd":"/view-only","MCPFacade":true}`},
		{"the tool tier", `{"cwd":"/view-only","ToolScope":"operator"}`},
		{"the profile grant", `{"cwd":"/capped","ProfileGranted":true,"ProfileId":"work"}`},
		{"the scrub stamp", `{"cwd":"/capped","EscalationScrubbed":[]}`},
		{"the audit id", `{"cwd":"/capped","DecisionID":"rd_forged"}`},
		{"screaming case", `{"cwd":"/capped","YOLOGRANTED":true}`},
		{"mixed case", `{"cwd":"/capped","yoloGRANTED":true}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if msg := spawnRefused(t, url, "tok-operator", tc.params, got); msg == "" {
				t.Fatal("refused with an empty error string")
			}
		})
	}
}

// TestTwoKeysThatFoldTogetherAreRefusedEvenWhenUnknown keeps the older
// ambiguity rule, and widens it: the guard and the decoder disagreeing is a
// refusal whether or not the hub has ever heard of the field.
func TestTwoKeysThatFoldTogetherAreRefusedEvenWhenUnknown(t *testing.T) {
	url, got, _ := ceilingServer(t, nil)
	for _, tc := range []struct{ name, params string }{
		{"a known field twice", `{"cwd":"/tmp","yoloGranted":true,"YoloGranted":true}`},
		{"a field the hub has never heard of", `{"cwd":"/tmp","somethingNew":1,"SomethingNew":2}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if msg := spawnRefused(t, url, "tok-operator", tc.params, got); !strings.Contains(msg, "case") {
				t.Errorf("refusal did not explain the ambiguity: %q", msg)
			}
		})
	}
}

// TestCanonicalSpellingsAndUnknownFieldsStillPass is the other half: the guard
// must not have made the spawn surface an allowlist. A field the hub does not
// know about is a provider's business, and it rides through untouched.
func TestCanonicalSpellingsAndUnknownFieldsStillPass(t *testing.T) {
	url, got, _ := ceilingServer(t, nil)
	m := spawnVia(t, url, "tok-operator",
		`{"cwd":"/tmp","model":"sonnet","toolScope":"operator","somethingTheHubHasNeverHeardOf":"keep"}`, got)
	if m["model"] != "sonnet" {
		t.Errorf("a canonical field was damaged: %v", m)
	}
	if m["somethingTheHubHasNeverHeardOf"] != "keep" {
		t.Errorf("an unknown field must ride through untouched: %v", m)
	}
}

// TestTheAliasRefusalCoversTheFederatedHop. The whole placement argument for
// this gate is that methodSanitizers is one table for call() and
// federatedCall(). A refusal that only bound locally would mean a capped hub
// could be walked around by asking a peer to do it.
func TestTheAliasRefusalCoversTheFederatedHop(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		if tok == "tok-operator" {
			return ScopedIdent{Scope: "operator", Methods: []string{spawnMethod}}, true
		}
		return ScopedIdent{}, false
	})
	fed := &fakeFed{got: make(chan json.RawMessage, 4)}
	srv.SetFederation(fed)

	caller := dialClientToken(t, url, "tok-operator")
	caller.send(Frame{Op: "call", ID: "f1", Method: "hub:work/agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp","YoloGranted":true}`)})
	f := caller.readUntil("error")
	if f.ID != "f1" {
		t.Fatalf("error frame id %q, want f1", f.ID)
	}
	select {
	case raw := <-fed.got:
		t.Fatalf("an aliased authority key crossed the federated hop: %s", raw)
	default:
	}
}

// TestNonObjectSpawnParamsStillPassThrough pins the documented exemption: there
// is no top-level key to alias in a scalar or an array, and the provider's own
// decoder rejects the shape.
func TestNonObjectSpawnParamsStillPassThrough(t *testing.T) {
	url, got, _ := ceilingServer(t, nil)
	caller := dialClientToken(t, url, "tok-operator")
	caller.send(Frame{Op: "call", ID: "s1", Method: "agents.spawn", Params: json.RawMessage(`["not-an-object"]`)})
	caller.readUntil("result")
	if raw := <-got; string(raw) != `["not-an-object"]` {
		t.Errorf("non-object params were rewritten: %s", raw)
	}
}
