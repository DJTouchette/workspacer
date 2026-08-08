package bus

import (
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// THE MATRIX. Round 6 fixed two topics by name and left the completeness
// question open; a scratch run of exactly this shape answered it — 23 of 25
// published topics were delivered to a `view` token, and the guard table had two
// rows. This is that run, kept.
//
// It drives the REAL bus with the REAL tiers, publishes every topic in the
// registry (plus one nobody classified), and asserts each delivery against the
// registry's own disposition. It cannot be satisfied by a table that has stopped
// being consulted, because the expectations come from the registry and the
// deliveries come from the socket.
func TestEveryClassifiedTopicIsDeliveredPerItsDisposition(t *testing.T) {
	url, _ := scopedServer(t)

	view := dialClientToken(t, url, "tok-view")
	if view.hello.Scope != "view" {
		t.Fatalf("hello scope = %q, want view", view.hello.Scope)
	}
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")

	// Publish every classified topic, then a sentinel the view tier must always
	// receive so the read loop terminates without a timeout.
	var probes []string
	for _, row := range capspec.EventTopics() {
		probe := row.Pattern
		if strings.HasSuffix(probe, "*") {
			probe = strings.TrimSuffix(probe, "*") + "PROBE"
		}
		probes = append(probes, probe)
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: probe,
			Data: json.RawMessage(`{"secret":"MATRIX"}`)}})
	}
	// The topic nobody classified. Under the old default this arrived; under a
	// closed default it must not.
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "invented.topic.nobody.classified"}})
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.state_changed",
		Data: json.RawMessage(`{"sentinel":true}`)}})

	got := map[string]bool{}
	for {
		f := view.readUntil("event")
		if f.Event == nil {
			t.Fatal("event frame with no envelope")
		}
		got[f.Event.Type] = true
		if f.Event.Type == "agent.state_changed" && strings.Contains(string(f.Event.Data), "sentinel") {
			break
		}
	}

	viewMethods := authtoken.ScopeView.Methods()
	checked := 0
	for i, row := range capspec.EventTopics() {
		probe := probes[i]
		checked++
		switch row.Disposition {
		case capspec.TopicOpenByDecision:
			if !got[probe] {
				t.Errorf("%q is open by decision and was NOT delivered to a view token — the fail-closed default has swallowed a feed the tier exists for (%s)", probe, row.Reason)
			}
		case capspec.TopicHostOnly:
			if got[probe] {
				t.Errorf("%q is host-only and a view token received it. %s", probe, row.Reason)
			}
		case capspec.TopicGuardedBy:
			// The expectation is computed from the TIER, not asserted flat:
			// a guarded topic whose capability the view tier DOES hold must
			// still arrive, or the guard is a blanket denial wearing a table.
			held := event.MatchesAny(viewMethods, row.Method)
			if held && !got[probe] {
				t.Errorf("%q is guarded by %q, which the view tier holds, and it was NOT delivered — the guard has stopped being a table and become a blanket", probe, row.Method)
			}
			if !held && got[probe] {
				t.Errorf("%q requires %q, which the view tier does not hold, and a view token received it anyway", probe, row.Method)
			}
		}
	}
	if checked < 20 {
		t.Fatalf("checked only %d topics — the registry shrank and this matrix is asserting almost nothing", checked)
	}
	if got["invented.topic.nobody.classified"] {
		t.Error("an UNCLASSIFIED topic was delivered to a view token. The event plane's default is open again, which is the state 23 of 25 topics shipped in — and no table of fixes can close a default.")
	}
	// FLOOR: the fix must not have simply muted the plane.
	if !got["agent.snapshot"] || !got["agent.state_changed"] {
		t.Fatal("the fleet feed did not reach the view tier — the remote clients this tier exists for are now blind")
	}
}

// The three proven pty.* escapes, each asserted by name, because they are
// siblings on ONE stream and the previous round guarded one of them.
func TestThePtyFamilyIsGuardedAsAFamily(t *testing.T) {
	url, _ := scopedServer(t)
	view := dialClientToken(t, url, "tok-view")
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	for _, typ := range []string{"pty.bytes.SECRET-42", "pty.exit", "pty.desync"} {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: typ,
			Data: json.RawMessage(`{"sessionId":"SECRET-42"}`)}})
	}
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.state_changed"}})

	f := view.readUntil("event")
	if f.Event == nil || f.Event.Type != "agent.state_changed" {
		t.Fatalf("a view token received %q — every pty.* topic is the output of sessions.attachTerminal, which this tier is refused", f.Event.Type)
	}

	// And the operator, who DOES hold the capability, still gets all three: a
	// guard that breaks the remote terminal is not a fix.
	op := dialClientToken(t, url, "tok-operator")
	op.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	op.readUntil("subscribed")
	for _, typ := range []string{"pty.bytes.SECRET-42", "pty.exit", "pty.desync"} {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: typ}})
		if f := op.readUntil("event"); f.Event == nil || f.Event.Type != typ {
			t.Fatalf("operator did not receive %q", typ)
		}
	}
}

// PROVEN LEAK, kept as a test. The delivery guard was a DELIVERY filter, not a
// subscription filter: broker.Publish matched pty.bytes.<id> and ENQUEUED it
// into the denied token's channel, and only the writer goroutine dropped it —
// after the channel had overflowed and sub.noteDrop had recorded the topic. That
// bookkeeping was then published back to the same connection as pty.desync,
// naming the sessionId of a stream it may not consume.
//
// The buffer size is the only thing this test changes; the real drop condition
// is an ordinary slow client (a phone on cellular).
func TestARefusedStreamLeavesNoDesyncTrail(t *testing.T) {
	srv := NewServer(broker.NewWithBuffer(1))
	url := serveTest(t, srv)
	srv.SetToken("host-secret")
	installScopedTiers(srv)

	view := dialClientToken(t, url, "tok-view")
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	for i := 0; i < 300; i++ {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-SESSION-42",
			Data: json.RawMessage(`"c2VjcmV0"`)}})
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.state_changed"}})
	}
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.snapshot",
		Data: json.RawMessage(`{"sentinel":true}`)}})

	for i := 0; i < 700; i++ {
		// Bounded: under a buffer of 1 the sentinel itself may be dropped, and a
		// test that then blocked forever would be a hang rather than a result.
		f, ok := view.tryReadUntil("event", "event", 500*time.Millisecond)
		if !ok {
			break
		}
		if f.Event == nil {
			continue
		}
		if strings.HasPrefix(f.Event.Type, "pty.") {
			t.Fatalf("LEAK: a view token received %s data=%s — the identity of a stream this connection was refused, arriving as the refused stream's own drop bookkeeping",
				f.Event.Type, f.Event.Data)
		}
		if f.Event.Type == "agent.snapshot" && strings.Contains(string(f.Event.Data), "sentinel") {
			return
		}
	}
	// Reaching here means the sentinel was itself dropped by the overflow, which
	// is legitimate under a buffer of 1 — the assertion above is the one that
	// matters and it ran on every frame that did arrive.
}

// FINDING 1, critical. eventTopicGuards was consulted on ONE arm of mayConsume
// and skipped on the other. A plugin with ZERO capability grants and
// `consumes: ["pty.bytes.*","fs.changed"]` was refused sessions.attachTerminal
// and fs.watch on the call plane and handed both capabilities' entire output on
// the event plane — the same crossing round 6 closed for scoped user tokens,
// still open for the plugin credential class. The install-consent dialog, the
// exemption's stated justification, rendered both consume lines at
// severity=normal.
func TestPluginConsumesCannotOutrunItsCapabilityGrants(t *testing.T) {
	url, srv := scopedServer(t, "sessions.attachTerminal", "fs.watch")
	srv.RegisterPluginToken("plug-tok", "evil.plugin", nil, capspec.EventGrants{
		Consumes: []string{"pty.bytes.*", "fs.changed", "plugin.log", "marker.done"},
	})

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	plug.readUntil("subscribed")

	// The call plane refuses both, asserted rather than assumed.
	for _, m := range []string{"sessions.attachTerminal", "fs.watch"} {
		plug.send(Frame{Op: "call", ID: m, Method: m, Params: json.RawMessage(`{"sessionId":"s1","path":"/tmp"}`)})
		if e := plug.readUntil("error"); !strings.Contains(e.Error, "not authorized") {
			t.Fatalf("%s error = %q, want the plugin capability refusal", m, e.Error)
		}
	}

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-42",
		Data: json.RawMessage(`"JCBjYXQgfi8uYXdzL2NyZWRlbnRpYWxz"`)}})
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "fs.changed",
		Data: json.RawMessage(`{"path":"/home/u/.ssh/id_ed25519","eventType":"change"}`)}})
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "plugin.log",
		Data: json.RawMessage(`{"line":"GITHUB_TOKEN=ghp_deadbeef"}`)}})
	// A topic the manifest declares that NOBODY classifies: the plugin must
	// still receive it, or the fix has broken the plugin event model.
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "marker.done"}})

	f := plug.readUntil("event")
	if f.Event == nil {
		t.Fatal("no event delivered")
	}
	if f.Event.Type != "marker.done" {
		t.Fatalf("a plugin holding NO capabilities received %q data=%s — its manifest string granted it the whole output of a capability the call plane had just refused it",
			f.Event.Type, f.Event.Data)
	}

	// And the mirror: a plugin that DOES hold sessions.attachTerminal receives
	// the stream, so the manifest stays a filter rather than becoming a blanket.
	srv.RegisterPluginToken("good-tok", "good.plugin", []capspec.Grant{{Method: "sessions.attachTerminal"}},
		capspec.EventGrants{Consumes: []string{"pty.bytes.*"}})
	good := dialClientToken(t, url, "good-tok")
	good.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	good.readUntil("subscribed")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-42"}})
	if f := good.readUntil("event"); f.Event == nil || f.Event.Type != "pty.bytes.SECRET-42" {
		t.Fatalf("a plugin granted sessions.attachTerminal did not receive the stream: %+v", f.Event)
	}
}

// FINDINGS 3 and 4, the publish direction. A plugin whose manifest declares
// `emits: ["layout.changed"]` or `["agent.snapshot"]` was publishing HOST STATE:
// a layout document every client adopts verbatim (carrying the four
// spawn-escalation fields layout.set scrubs, plus a publisher-chosen version
// that wins every later comparison), and the snapshot internal/push turns into
// the phone's "needs you" lock-screen notification.
func TestNobodyButTheHostMayPublishHostState(t *testing.T) {
	url, srv := scopedServer(t)
	srv.RegisterPluginToken("plug-tok", "evil.plugin", nil, capspec.EventGrants{
		Emits: []string{"layout.changed", "agent.snapshot", "plugin.settings.changed", "myplugin.tick"},
	})

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	host.readUntil("subscribed")

	plug := dialClientToken(t, url, "plug-tok")
	for _, typ := range []string{"layout.changed", "agent.snapshot", "plugin.settings.changed"} {
		plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: typ,
			Data: json.RawMessage(`{"forged":true}`)}})
		e := plug.readUntil("error")
		if !strings.Contains(e.Error, "not authorized") {
			t.Fatalf("publishing %q as a plugin returned %q, want a refusal — every one of these topics is read by a trusted in-hub consumer as authoritative host state", typ, e.Error)
		}
	}
	// Its OWN topic still works: plugins emit their own events, and classifying
	// host topics must not take that away.
	plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: "myplugin.tick"}})
	if f := host.readUntil("event"); f.Event == nil || f.Event.Type != "myplugin.tick" {
		t.Fatalf("a plugin's own declared topic did not publish: %+v", f.Event)
	}

	// A scoped user token cannot publish at all, in either direction.
	viewc := dialClientToken(t, url, "tok-view")
	viewc.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.snapshot"}})
	if e := viewc.readUntil("error"); !strings.Contains(e.Error, "not authorized") {
		t.Fatalf("a view token published agent.snapshot: %q", e.Error)
	}
}

// installScopedTiers wires the REAL authtoken tiers into a server, so every
// test in this package that talks about "view" or "triage" is talking about the
// tier the product actually mints rather than a list a test invented.
func installScopedTiers(srv *Server) {
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		switch tok {
		case "tok-view":
			return ScopedIdent{Scope: "view", Methods: authtoken.ScopeView.Methods()}, true
		case "tok-triage":
			return ScopedIdent{Scope: "triage", Methods: authtoken.ScopeTriage.Methods()}, true
		case "tok-operator":
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		}
		return ScopedIdent{}, false
	})
}

// serveTest exposes a server whose broker the caller built (the desync test
// needs a buffer of 1), which rpcServerWith cannot do.
func serveTest(t *testing.T, srv *Server) string {
	t.Helper()
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	return hs.URL
}

// The bus must actually INSTALL the admission filter. The broker's own test
// pins the filter's behaviour, and the desync test pins the wire, but both stay
// green with `broker.Subscribe(nil)` back in place: the writer-side mayConsume
// check hides the wiring by refusing the same events one step later. What it
// cannot hide is the COST — an event that reached the channel took a slot and,
// past capacity, was recorded as a drop, and that record is what escaped as
// pty.desync.
//
// So this asserts the negative directly: a refused topic must leave the denied
// connection's drop count at zero, however much of it is published.
func TestTheBusFiltersRefusedTopicsBeforeTheyReachTheChannel(t *testing.T) {
	b := broker.NewWithBuffer(1)
	srv := NewServer(b)
	url := serveTest(t, srv)
	srv.SetToken("host-secret")
	installScopedTiers(srv)

	view := dialClientToken(t, url, "tok-view")
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	// Enough that an unfiltered path cannot keep up: the reader is idle, so the
	// socket backs up, the writer goroutine blocks, and a channel of capacity 1
	// starts discarding — which is exactly the production condition (a phone on
	// cellular) the desync signal exists for.
	for i := 0; i < 20000; i++ {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-42",
			Data: json.RawMessage(`"c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0"`)}})
	}
	// Let the fan-out settle: publishes are async relative to this goroutine.
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) && b.SubscriberCount() > 0 {
		if n := b.DroppedTotal(); n > 0 {
			t.Fatalf("a topic the view tier may not consume was enqueued and then discarded %d times. It cost this connection buffer capacity it was never entitled to, and each discard on a stream topic is remembered BY NAME — which is the record that escaped as a pty.desync event naming the sessionId.", n)
		}
		time.Sleep(50 * time.Millisecond)
	}
	if n := b.DroppedTotal(); n != 0 {
		t.Fatalf("drop count = %d, want 0", n)
	}
	view.ws.CloseNow()
}
