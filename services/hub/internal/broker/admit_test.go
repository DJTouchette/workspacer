package broker

import (
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// AUTHORIZATION APPLIED AFTER THE FAN-OUT IS NOT AUTHORIZATION.
//
// The bus checked its consume grant in the WRITER goroutine, so a denied topic
// had already been matched, enqueued into the denied subscriber's channel, and —
// when that channel filled — recorded as a DROP against it. For a
// pty.bytes.<id> stream that bookkeeping was then published back to the very
// connection that may not consume the stream, as a pty.desync event naming the
// sessionId. The topic-guard row for pty.desync stops the leak; this stops the
// state that produced it from existing.
//
// Asserted on the bookkeeping directly, because that is the thing the leak was
// made of: a refused topic must cost the subscriber no slot and leave no trace.
func TestARefusedTopicCostsNoBufferAndLeavesNoDropRecord(t *testing.T) {
	b := NewWithBuffer(1)
	sub := b.SubscribeFiltered([]string{"*"}, func(typ string) bool {
		return typ != "pty.bytes.SECRET-42"
	})
	defer b.Unsubscribe(sub)

	for i := 0; i < 200; i++ {
		b.Publish(event.Envelope{Type: "pty.bytes.SECRET-42"})
	}
	if n := sub.Dropped(); n != 0 {
		t.Errorf("a refused topic was enqueued and then dropped %d times — it took this subscriber's buffer capacity, and on a stream topic each drop is remembered BY NAME", n)
	}
	if d := sub.TakeDesyncs(); len(d) != 0 {
		t.Errorf("the refused stream left a desync record %v — that record is what escaped to the denied connection as a pty.desync event naming the sessionId", d)
	}

	// FLOOR: the filter must not be a mute. The one event it admits arrives.
	b.Publish(event.Envelope{Type: "agent.state_changed"})
	select {
	case ev := <-sub.C:
		if ev.Type != "agent.state_changed" {
			t.Fatalf("first delivered event = %q; the refused stream occupied the buffer after all", ev.Type)
		}
	default:
		t.Fatal("nothing was delivered — the admission filter is refusing everything")
	}
}

// A nil filter is the unfiltered subscription every existing caller has, and it
// must still count drops: the fix must not have turned the drop accounting off
// for everyone.
func TestAnUnfilteredSubscriptionStillRecordsDrops(t *testing.T) {
	b := NewWithBuffer(1)
	sub := b.Subscribe([]string{"*"})
	defer b.Unsubscribe(sub)
	for i := 0; i < 10; i++ {
		b.Publish(event.Envelope{Type: "pty.bytes.s1"})
	}
	if sub.Dropped() == 0 {
		t.Fatal("an unfiltered slow subscriber recorded no drops — the desync signal a real terminal client depends on is gone")
	}
	if d := sub.TakeDesyncs(); len(d) != 1 || d[0] != "pty.bytes.s1" {
		t.Fatalf("desync topics = %v, want [pty.bytes.s1]", d)
	}
}
