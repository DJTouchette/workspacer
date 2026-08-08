package bus

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// REVOCATION ON THE EVENT PLANE, which no test could see.
//
// Three tests already cover revocation (revoke_e2e_test.go) and all three
// observe the CLOSE: `if !caller.trySend(...) { return }`, or a read that fails
// in milliseconds. CloseNow is synchronous, so it always wins the race in a
// test, and that made `if cn.revoked.Load() { return false }` deletable from
// BOTH mayConsume and mayPublish with every package green — while the flag's own
// doc comment says it exists precisely for the interleaving where the close has
// NOT landed yet: "revoked is what makes the NEXT call on an in-flight
// connection fail even before the close lands".
//
// That interleaving is real and it is not narrow. UnregisterPluginToken flags
// and closes under ptMu, but the read loop, the writer goroutine and the router
// are three other goroutines already running: a publish frame read off the wire,
// or an event handed to the pump, is mid-flight when the flag lands. This test
// is that moment with no socket in the way — flag set, close not yet applied —
// and it asserts the answer on both directions of the plane.
func TestARevokedConnectionMayNeitherPublishNorConsume(t *testing.T) {
	t.Run("plugin", func(t *testing.T) {
		cn := &conn{
			pluginID: "notes",
			emits:    []string{"notes.saved"},
			consumes: []string{"notes.saved", "marker.done"},
		}
		if !cn.mayPublish("notes.saved") {
			t.Fatal("floor: a live plugin must be able to publish its declared topic")
		}
		if !cn.mayConsume("marker.done") {
			t.Fatal("floor: a live plugin must receive a topic its manifest declares")
		}
		cn.revoked.Store(true)
		if cn.mayPublish("notes.saved") {
			t.Error("a REVOKED plugin connection may still publish. Its next frame is already read off the wire when the flag lands, and a publish from a credential the host has dropped is a write to every subscriber on the bus.")
		}
		if cn.mayConsume("marker.done") {
			t.Error("a REVOKED plugin connection is still delivered events. The socket sits there consuming the firehose until the close lands, and agent.* alone carries session ids, cwds and transcript-adjacent state.")
		}
	})

	t.Run("scoped user token", func(t *testing.T) {
		cn := &conn{scope: "view", scopeMethods: authtoken.ScopeView.Methods()}
		if !cn.mayConsume("agent.state_changed") {
			t.Fatal("floor: a live view token must receive the fleet feed")
		}
		cn.revoked.Store(true)
		if cn.mayConsume("agent.state_changed") {
			t.Error("a REVOKED scoped token is still delivered the fleet feed")
		}
	})

	// Revocation outranks TRUSTED on both, and for the same reason the mayCall
	// ordering test gives: an operator-scoped token is PROMOTED to trusted at the
	// handshake, so a revoked operator socket that kept publishing and consuming
	// everything is the worst case of the hole the flag exists for. Moving either
	// check below its trusted short-circuit is the obvious refactor that would
	// silently undo this.
	t.Run("trusted, promoted from an operator token", func(t *testing.T) {
		cn := &conn{trusted: true}
		if !cn.mayPublish("agent.snapshot") || !cn.mayConsume("pty.bytes.S1") {
			t.Fatal("floor: a trusted connection publishes and consumes anything")
		}
		cn.revoked.Store(true)
		if cn.mayPublish("agent.snapshot") {
			t.Error("the revoked check is below mayPublish's trusted short-circuit — a revoked operator socket can still forge host state")
		}
		if cn.mayConsume("pty.bytes.S1") {
			t.Error("the revoked check is below mayConsume's trusted short-circuit — a revoked operator socket still receives every guarded topic, PTY streams included")
		}
	})
}

// THE SECOND LAYER, pinned rather than assumed.
//
// Since SubscribeFiltered the consume grant is applied at ENQUEUE, so the
// delivery-time mayConsume in pumpEvents refuses events that, on the happy path,
// were never enqueued — deleting it left every suite green. It is kept as
// defence in depth for ONE stated reason, and this is that reason made
// executable: the admission filter is a snapshot of an authorization that can
// change under a live subscription, and an event admitted before `revoked`
// landed is already sitting in sub.C when it does.
//
// The subscription here carries NO admission filter, which is exactly the state
// the first layer's absence or staleness produces. What the pump does with it is
// the whole claim.
func TestTheWriterLoopReChecksTheConsumeGrantAtDeliveryTime(t *testing.T) {
	cn := &conn{scope: "view", scopeMethods: authtoken.ScopeView.Methods()}
	b := broker.New()
	sub := b.Subscribe([]string{"*"}) // the enqueue layer, absent

	sent, done := pumpInto(cn, sub)
	b.Publish(event.Envelope{Type: "pty.bytes.SECRET-42", Data: json.RawMessage(`"JCBjYXQgfi8uc3NoL2lkX2Vk"`)})
	b.Publish(event.Envelope{Type: "agent.state_changed", Data: json.RawMessage(`{"sentinel":true}`)})
	b.Unsubscribe(sub)
	<-done

	for _, f := range *sent {
		if f.Event != nil && strings.HasPrefix(f.Event.Type, "pty.") {
			t.Fatalf("the writer wrote %s data=%s to a view token. pty.* is the output of sessions.attachTerminal, which this tier is refused — and once the enqueue filter is stale or gone, this check is the only thing between the two planes.",
				f.Event.Type, f.Event.Data)
		}
	}
	// FLOOR: the pump must still be a pump.
	if len(*sent) != 1 || (*sent)[0].Event == nil || (*sent)[0].Event.Type != "agent.state_changed" {
		t.Fatalf("the fleet feed did not reach the view token: %+v", *sent)
	}
}

// THE DESYNC FRAME IS NOT FILTERED BY ANYTHING ELSE. It is SYNTHESISED in the
// pump from sub.TakeDesyncs(), so it never passed the broker at all and the
// enqueue-time admission filter never saw it. Round 7 closed the leak by not
// enqueuing refused topics; this is the other side of that door — the drop
// bookkeeping of a guarded stream, turned into an event naming the sessionId,
// written straight to the socket.
//
// The state below is the production one, reached the production way: a stream
// topic overflows a slow consumer's buffer, and the broker records the topic by
// name.
func TestADesyncReportIsRefusedToAConnectionThatMayNotConsumeTheStream(t *testing.T) {
	b := broker.NewWithBuffer(1)
	sub := b.Subscribe([]string{"*"})

	// Overflow: one event fits, the rest are dropped and remembered BY NAME
	// because pty.bytes.* is a stream topic.
	for i := 0; i < 5; i++ {
		b.Publish(event.Envelope{Type: "pty.bytes.SECRET-42", Data: json.RawMessage(`"c2VjcmV0"`)})
	}
	if b.DroppedTotal() == 0 {
		t.Fatal("precondition: the buffer did not overflow, so there is no desync record to leak")
	}
	<-sub.C // clear the slot by hand so the pump starts on a frame it may deliver
	b.Publish(event.Envelope{Type: "agent.state_changed"})

	cn := &conn{scope: "view", scopeMethods: authtoken.ScopeView.Methods()}
	sent, done := pumpInto(cn, sub)
	b.Unsubscribe(sub)
	<-done

	for _, f := range *sent {
		if f.Event == nil {
			continue
		}
		if f.Event.Type == "pty.desync" || strings.Contains(string(f.Event.Data), "SECRET-42") {
			t.Fatalf("a view token was told %s data=%s — pty.desync is guarded by sessions.attachTerminal, which this tier does not hold, and the payload IS the sessionId of a stream it may not consume. The frame is built inside the pump, so no enqueue filter can catch it.",
				f.Event.Type, f.Event.Data)
		}
	}

	// FLOOR, and it is the point of the feature: a connection that DOES hold the
	// stream must still be told to re-attach, or a dropped PTY chunk silently
	// corrupts its terminal forever.
	b2 := broker.NewWithBuffer(1)
	sub2 := b2.Subscribe([]string{"*"})
	for i := 0; i < 5; i++ {
		b2.Publish(event.Envelope{Type: "pty.bytes.SECRET-42"})
	}
	<-sub2.C
	b2.Publish(event.Envelope{Type: "agent.state_changed"})
	sent2, done2 := pumpInto(&conn{trusted: true}, sub2)
	b2.Unsubscribe(sub2)
	<-done2

	desynced := false
	for _, f := range *sent2 {
		if f.Event != nil && f.Event.Type == "pty.desync" && strings.Contains(string(f.Event.Data), "SECRET-42") {
			desynced = true
		}
	}
	if !desynced {
		t.Fatalf("an operator connection lost PTY bytes and was never told: %+v — the guard has become a blanket and the remote terminal renders garbage with neither side knowing", *sent2)
	}
}

// pumpInto runs cn.pumpEvents against sub with a recording sink. The returned
// slice is safe to read once done is closed (the pump goroutine has returned,
// which happens-before the receive).
func pumpInto(cn *conn, sub *broker.Subscription) (*[]Frame, chan struct{}) {
	var sent []Frame
	done := make(chan struct{})
	go func() {
		defer close(done)
		cn.pumpEvents(sub, func(f Frame) error {
			sent = append(sent, f)
			return nil
		})
	}()
	return &sent, done
}
