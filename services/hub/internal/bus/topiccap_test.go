package bus

import (
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// bigTopicList builds n distinct patterns — the payload of the availability
// finding: one ~1 MB frame of them.
func bigTopicList(n int) []string {
	out := make([]string, n)
	for i := range out {
		out[i] = "topic." + strconv.Itoa(i)
	}
	return out
}

// TestOversizedSubscribeIsRejected covers the one op with no authorization
// check at all. `subscribe` is deliberately open to every tier, so the lowest-
// privilege credential the system mints — a read-only `view` token — is the one
// used here: it must not be able to hand the broker an unbounded topic list,
// because rewriting that list takes the lock every publisher's fan-out needs.
// The frame is refused outright rather than truncated, and the connection stays
// usable afterwards.
func TestOversizedSubscribeIsRejected(t *testing.T) {
	url, _ := scopedServer(t)
	c := dialClientToken(t, url, "tok-view")

	c.send(Frame{Op: "subscribe", Topics: bigTopicList(maxFrameTopics + 1)})
	errFrame := c.readUntil("error")
	if !strings.Contains(errFrame.Error, "over the") {
		t.Fatalf("error = %q, want it to name the per-frame topic limit", errFrame.Error)
	}

	// Rejected means rejected: none of those patterns took effect, so a normal
	// subscribe that follows sees only its own topic.
	c.send(Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	got := c.readUntil("subscribed").Topics
	if len(got) != 1 || got[0] != "agent.*" {
		t.Fatalf("topics after a rejected oversized subscribe = %v, want [agent.*]", got)
	}
}

// unsubscribe rewrites the same list under the same lock, so a subscribe-only
// cap would leave the stall wide open through the other door.
func TestOversizedUnsubscribeIsRejected(t *testing.T) {
	url, _ := scopedServer(t)
	c := dialClientToken(t, url, "tok-view")

	c.send(Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	c.readUntil("subscribed")

	c.send(Frame{Op: "unsubscribe", Topics: bigTopicList(maxFrameTopics + 1)})
	errFrame := c.readUntil("error")
	if !strings.Contains(errFrame.Error, "unsubscribe") {
		t.Fatalf("error = %q, want it to name the rejected op", errFrame.Error)
	}

	c.send(Frame{Op: "unsubscribe", Topics: []string{"nothing.matching"}})
	if got := c.readUntil("unsubscribed").Topics; len(got) != 1 || got[0] != "agent.*" {
		t.Fatalf("topics after a rejected oversized unsubscribe = %v, want [agent.*] intact", got)
	}
}

// A subscribe frame at the limit is ordinary traffic and must still work end to
// end — the cap is an availability bound, not a new authorization rule.
func TestSubscribeAtTheLimitStillDelivers(t *testing.T) {
	b := broker.New()
	hs := httptest.NewServer(NewServer(b).Handler())
	t.Cleanup(hs.Close)
	c := dialClient(t, hs.URL)

	topics := bigTopicList(maxFrameTopics - 1)
	topics = append(topics, "agent.*")
	c.send(Frame{Op: "subscribe", Topics: topics})
	if got := len(c.readUntil("subscribed").Topics); got != maxFrameTopics {
		t.Fatalf("retained %d topics, want all %d", got, maxFrameTopics)
	}

	b.Publish(event.Envelope{Type: "agent.spawned"})
	if ev := c.readUntil("event"); ev.Event == nil || ev.Event.Type != "agent.spawned" {
		t.Fatalf("subscription at the limit did not deliver: %+v", ev.Event)
	}
}
