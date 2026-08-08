package bus

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// The event-side of plugin authorization: a per-plugin token may publish only
// the types it declared in `emits`, receive only those in `consumes`, and
// register as a provider only for methods in `provides`. Trusted (host) conns
// bypass all three. These mirror the capability-call enforcement in rpc_test.go.

// A plugin can publish an event type its manifest declared.
func TestPublishAllowedByEmits(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", nil, capspec.EventGrants{
		Emits: []string{"example.clock.*"},
	})

	// A trusted subscriber receives the published event, proving it went through.
	sub := dialClientToken(t, url, "host-secret")
	sub.send(Frame{Op: "subscribe", Topics: []string{"example.clock.*"}})
	sub.readUntil("subscribed")

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: "example.clock.tick"}})

	if ev := sub.readUntil("event"); ev.Event == nil || ev.Event.Type != "example.clock.tick" {
		t.Fatalf("subscriber got %+v, want an example.clock.tick event", ev.Event)
	}
}

// A plugin publishing an undeclared type is rejected — in particular it cannot
// publish a command.* event to drive the app without the grant.
func TestPublishDeniedWithoutEmits(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", nil, capspec.EventGrants{
		Emits: []string{"example.clock.*"}, // NOT command.*
	})

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: "command.spawn_agent"}})
	e := plug.readUntil("error")
	if !strings.Contains(e.Error, "not authorized to publish") {
		t.Fatalf("error = %q, want it to mention publish authorization", e.Error)
	}
}

// A broad subscribe cannot widen a plugin past its `consumes`: it receives the
// declared type but not others published on the same broker.
func TestConsumeFilterCapsDeliveryToDeclared(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", nil, capspec.EventGrants{
		Consumes: []string{"agent.*"},
	})

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "subscribe", Topics: []string{"*"}}) // asks for everything
	plug.readUntil("subscribed")

	// The host publishes one allowed and one disallowed event, in order.
	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "secret.leak"}})
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.state_changed"}})

	// The first event the plugin receives must be the allowed one — the
	// disallowed `secret.leak` is dropped at delivery despite the "*" subscribe.
	got := plug.readUntil("event")
	if got.Event == nil || got.Event.Type != "agent.state_changed" {
		t.Fatalf("delivered %+v, want agent.state_changed (secret.leak must be filtered)", got.Event)
	}
}

// A trusted connection is unrestricted on the event side.
func TestTrustedConnBypassesEventGrants(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")

	sub := dialClientToken(t, url, "host-secret")
	sub.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	sub.readUntil("subscribed")

	pub := dialClientToken(t, url, "host-secret")
	pub.send(Frame{Op: "publish", Event: &event.Envelope{Type: "command.spawn_agent"}})
	if ev := sub.readUntil("event"); ev.Event == nil || ev.Event.Type != "command.spawn_agent" {
		t.Fatalf("trusted publish/subscribe failed: %+v", ev.Event)
	}
}

// register is gated by `provides`: a plugin may register the methods it declared
// and only those; the ack reflects what was actually registered.
func TestRegisterGatedByProvides(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", nil, capspec.EventGrants{
		Provides: []string{"recon.*"},
	})

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "register", Methods: []string{"recon.overview", "agents.spawn"}})
	ack := plug.readUntil("registered")
	if len(ack.Methods) != 1 || ack.Methods[0] != "recon.overview" {
		t.Fatalf("registered = %v, want only [recon.overview]", ack.Methods)
	}

	// The withheld method has no provider, so a call to it finds nobody home.
	caller := dialClientToken(t, url, "host-secret")
	caller.send(Frame{Op: "call", ID: "c1", Method: "agents.spawn"})
	if e := caller.readUntil("error"); !strings.Contains(e.Error, "no provider") {
		t.Fatalf("error = %q, want 'no provider' for the ungranted method", e.Error)
	}
}

// A plugin that declared no provider surface registers nothing.
func TestRegisterDeniedWithoutProvides(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", nil, capspec.EventGrants{})

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "register", Methods: []string{"recon.overview"}})
	if ack := plug.readUntil("registered"); len(ack.Methods) != 0 {
		t.Fatalf("registered = %v, want none (no provides grant)", ack.Methods)
	}
}

// ── The event plane and the capability plane must agree ─────────────────────

// TestScopedTokenCannotReceivePtyBytesItMayNotAttachTo is the composition test
// for two authorization planes that answered the same question differently.
//
// The CAPABILITY plane refuses `sessions.attachTerminal` to a view token: it is
// sensitive:true in CAP_LABELS and in neither viewMethods nor triageMethods. The
// EVENT plane read `cn.trusted || cn.scopeMethods != nil || MatchesAny(...)`,
// whose middle clause waved EVERY topic through for ANY scoped user token
// without consulting the allowlist that had just denied the call.
//
// So the view token subscribed to `pty.bytes.<id>` and received the session's
// raw PTY bytes — the whole screen, ring-buffer replay included, since attaching
// deliberately restarts the stream. terminals.* is absent from both scoped tiers
// entirely, so no view or triage METHOD reaches a terminal's screen: the event
// plane was the only door, and it was open.
//
// Neither half is wrong alone. The attach is correctly refused; event
// subscriptions are deliberately part of even the view tier. What was missing
// was that a topic carrying a capability's OUTPUT requires that capability.
func TestScopedTokenCannotReceivePtyBytesItMayNotAttachTo(t *testing.T) {
	url, _ := scopedServer(t)

	view := dialClientToken(t, url, "tok-view")
	hello := view.hello
	if hello.Scope != "view" {
		t.Fatalf("hello scope = %q, want view — the harness is not minting the tier this test is about", hello.Scope)
	}

	// STEP 1 — the capability plane says no. Asserted, not assumed: if a future
	// tier admitted the method, the delivery below would be legitimate and this
	// test would be pinning the wrong thing.
	view.send(Frame{Op: "call", ID: "att", Method: "sessions.attachTerminal",
		Params: json.RawMessage(`{"sessionId":"s1"}`)})
	if e := view.readUntil("error"); !strings.Contains(e.Error, "outside this token's") {
		t.Fatalf("sessions.attachTerminal error = %q, want the scope refusal", e.Error)
	}

	// STEP 2 — the event plane. "*" is the widest ask a client can make.
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.s1",
		Data: json.RawMessage(`{"data":"c3NoLWtleXNjYW4gc2VjcmV0"}`)}})
	// A topic that is NOT the output of any capability, published after it, so
	// the read below cannot block forever and so the FLOOR is asserted in the
	// same run: the fleet feed a view token exists to receive must still arrive.
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.state_changed"}})

	got := view.readUntil("event")
	if got.Event == nil {
		t.Fatal("no event delivered at all")
	}
	if strings.HasPrefix(got.Event.Type, "pty.bytes.") {
		t.Fatalf("a %q token received %s — the capability plane refused sessions.attachTerminal to this exact credential and the event plane delivered its entire output anyway",
			hello.Scope, got.Event.Type)
	}
	if got.Event.Type != "agent.state_changed" {
		t.Fatalf("first delivered event = %q, want agent.state_changed", got.Event.Type)
	}
}

// The mirror image, and the reason the guard is a TABLE rather than a blanket
// refusal: a tier that DOES hold the capability must still receive its output,
// or the fix silently breaks the remote terminal view for the operator.
func TestOperatorTokenStillReceivesPtyBytes(t *testing.T) {
	url, _ := scopedServer(t)

	op := dialClientToken(t, url, "tok-operator")
	op.send(Frame{Op: "subscribe", Topics: []string{"pty.bytes.*"}})
	op.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.s1"}})

	if got := op.readUntil("event"); got.Event == nil || got.Event.Type != "pty.bytes.s1" {
		t.Fatalf("an operator token did not receive pty.bytes: %+v", got.Event)
	}
}

// Every guarded topic must name a method capspec has classified, and every
// guarded method must be one a scoped tier could plausibly be denied — a table
// naming a method nobody registers guards nothing, and would be indistinguishable
// from a typo.
func TestEventTopicGuardsNameClassifiedMethods(t *testing.T) {
	guards := capspec.EventTopicGuards()
	if len(guards) == 0 {
		t.Fatal("the topic guard table is empty — mayConsume's scoped arm is unconditional again")
	}
	for topic, method := range guards {
		if capspec.MissingClassification(method) {
			t.Errorf("topic %q is guarded by %q, which capspec says nothing about — a guard pointing at an unclassified method cannot be reasoned about", topic, method)
		}
		// The lookup must match the pattern's own shape: a trailing '*' matches
		// any suffix, a bare topic matches itself exactly.
		probe := topic
		if strings.HasSuffix(topic, "*") {
			probe = strings.TrimSuffix(topic, "*") + "anything"
		}
		if m, ok := capspec.EventTopicCapability(probe); !ok || m != method {
			t.Errorf("EventTopicCapability(%q) = (%q,%v), want (%q,true) — the table does not match its own pattern %q", probe, m, ok, method, topic)
		}
	}
	// An ordinary topic must NOT be guarded, or every tier loses every feed.
	if _, guarded := capspec.EventTopicCapability("agent.state_changed"); guarded {
		t.Error("agent.state_changed is guarded — the table is matching everything, which takes the fleet feed away from the view tier it exists for")
	}
}
