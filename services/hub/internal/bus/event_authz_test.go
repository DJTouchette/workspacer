package bus

import (
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
