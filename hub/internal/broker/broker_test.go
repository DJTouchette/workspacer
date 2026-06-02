package broker

import (
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

func recv(t *testing.T, sub *Subscription) (event.Envelope, bool) {
	t.Helper()
	select {
	case ev, ok := <-sub.C:
		return ev, ok
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for event")
		return event.Envelope{}, false
	}
}

func TestPublishReachesMatchingSubscriber(t *testing.T) {
	b := New()
	sub := b.Subscribe([]string{"agent.*"})
	b.Publish(event.Envelope{Type: "agent.spawned"})
	ev, _ := recv(t, sub)
	if ev.Type != "agent.spawned" {
		t.Fatalf("got %q", ev.Type)
	}
}

func TestNonMatchingNotDelivered(t *testing.T) {
	b := New()
	sub := b.Subscribe([]string{"agent.*"})
	b.Publish(event.Envelope{Type: "git.changed"})
	select {
	case ev := <-sub.C:
		t.Fatalf("unexpected delivery: %q", ev.Type)
	case <-time.After(50 * time.Millisecond):
	}
}

func TestFanOutToAllSubscribers(t *testing.T) {
	b := New()
	a := b.Subscribe([]string{"*"})
	c := b.Subscribe([]string{"*"})
	b.Publish(event.Envelope{Type: "session.usage"})
	if ev, _ := recv(t, a); ev.Type != "session.usage" {
		t.Errorf("a got %q", ev.Type)
	}
	if ev, _ := recv(t, c); ev.Type != "session.usage" {
		t.Errorf("c got %q", ev.Type)
	}
}

func TestUnsubscribeStopsDeliveryAndCloses(t *testing.T) {
	b := New()
	sub := b.Subscribe([]string{"*"})
	b.Unsubscribe(sub)
	if _, ok := <-sub.C; ok {
		t.Fatal("channel should be closed after Unsubscribe")
	}
	b.Publish(event.Envelope{Type: "agent.spawned"}) // must not panic
	if n := b.SubscriberCount(); n != 0 {
		t.Fatalf("SubscriberCount=%d want 0", n)
	}
}

func TestAddAndRemoveTopics(t *testing.T) {
	b := New()
	sub := b.Subscribe(nil)
	sub.AddTopics("agent.*", "agent.*") // dedup
	if got := sub.Topics(); len(got) != 1 {
		t.Fatalf("topics=%v want one", got)
	}
	b.Publish(event.Envelope{Type: "agent.done"})
	if ev, _ := recv(t, sub); ev.Type != "agent.done" {
		t.Fatalf("got %q", ev.Type)
	}
	sub.RemoveTopics("agent.*")
	b.Publish(event.Envelope{Type: "agent.done"})
	select {
	case <-sub.C:
		t.Fatal("should not receive after RemoveTopics")
	case <-time.After(50 * time.Millisecond):
	}
}

// The smoothness guarantee: a slow consumer never blocks the publisher.
func TestSlowConsumerDropsRatherThanBlocks(t *testing.T) {
	b := NewWithBuffer(1)
	sub := b.Subscribe([]string{"*"})
	done := make(chan struct{})
	go func() {
		for i := 0; i < 1000; i++ { // never drained
			b.Publish(event.Envelope{Type: "spam"})
		}
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("Publish blocked on a slow consumer")
	}
	if sub.Dropped() == 0 {
		t.Fatal("expected drops on an undrained subscriber")
	}
}
