package bus

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

// DEMAND SIGNALLING, END TO END (see demand.go for the design).
//
// The contract under test: a provider that sends {op:"demand", topics:[prefix]}
// hears {op:"demand", topic, demand:true|false} on each 0↔1 transition of the
// EXACT-topic subscriber count under that prefix — including the release the
// hub performs itself when a subscriber's socket dies. The rules that keep it
// honest each get a failing test: wildcards never create demand, a repeat
// subscribe never double-counts, an unentitled subscriber never counts at all,
// and an unentitled watcher never hears.

// readDemand reads frames until a demand frame arrives.
func (c *client) readDemand() Frame {
	return c.readUntil("demand")
}

// expectNoDemand fails if a demand frame arrives within the window. 250ms is
// enough for the synchronous deliver() path — frames are sent before the
// subscriber's own "subscribed" ack, so by the time a test gets here any frame
// that was going to be sent already has been.
//
// TERMINAL: coder/websocket closes the connection when a read context expires,
// so this must be the LAST thing a test does with the connection. Mid-test
// negatives are proven by ordering instead — deliver() runs before the
// subscriber's "subscribed" ack, so "the next demand frame is the sentinel"
// proves nothing was sent before it.
func (c *client) expectNoDemand() {
	c.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 250*time.Millisecond)
	defer cancel()
	for {
		_, data, err := c.ws.Read(ctx)
		if err != nil {
			return // timeout = success
		}
		var f Frame
		_ = json.Unmarshal(data, &f)
		if f.Op == "demand" {
			c.t.Fatalf("unexpected demand frame: %+v", f)
		}
	}
}

func TestDemandAnnouncesZeroOneTransitionsOnly(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	provider.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})

	sub1 := dialClient(t, url)
	sub2 := dialClient(t, url)

	// First subscriber: 0→1, announced.
	sub1.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub1.readUntil("subscribed")
	f := provider.readDemand()
	if f.Topic != "agent.conversation.s1" || !f.Demand {
		t.Fatalf("want demand:true for s1, got %+v", f)
	}

	// Second subscriber on the same topic: 1→2, silent. Proven by ordering —
	// the next demand frame the provider sees is for a DIFFERENT topic
	// subscribed afterwards, and deliver() runs before the subscribe ack.
	sub2.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub2.readUntil("subscribed")
	sub1.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s2"}})
	sub1.readUntil("subscribed")
	f = provider.readDemand()
	if f.Topic != "agent.conversation.s2" || !f.Demand {
		t.Fatalf("a 1→2 transition leaked (or s2 was missed): %+v", f)
	}

	// One of two unsubscribing: 2→1, silent — the stream must survive for the
	// other watcher. Again proven by ordering with a sentinel topic.
	sub1.send(Frame{Op: "unsubscribe", Topics: []string{"agent.conversation.s1"}})
	sub1.readUntil("unsubscribed")
	sub1.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s3"}})
	sub1.readUntil("subscribed")
	f = provider.readDemand()
	if f.Topic != "agent.conversation.s3" || !f.Demand {
		t.Fatalf("a 2→1 transition announced a release out from under a live subscriber: %+v", f)
	}

	// The last subscriber's SOCKET dying is the release — no unsubscribe frame,
	// no lease timer. handleBus's defer must hand the release to the watcher.
	sub2.ws.CloseNow()
	f = provider.readDemand()
	if f.Topic != "agent.conversation.s1" || f.Demand {
		t.Fatalf("want demand:false for s1 after its last subscriber's socket died, got %+v", f)
	}
}

func TestDemandIgnoresWildcardSubscribers(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	provider.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})

	sub := dialClient(t, url)
	// The desktop subscribes to "*". If a wildcard counted, one connected
	// client would conjure a transcript firehose for every session on the bus —
	// and "agent.conversation.*" falls under the watched prefix, so a counted
	// wildcard would have produced a demand frame for it here.
	sub.send(Frame{Op: "subscribe", Topics: []string{"*", "agent.*", "agent.conversation.*"}})
	sub.readUntil("subscribed")

	// An exact topic from the same connection still counts, and is the FIRST
	// demand frame the provider sees (ordering proves the wildcards sent none).
	sub.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("subscribed")
	f := provider.readDemand()
	if f.Topic != "agent.conversation.s1" || !f.Demand {
		t.Fatalf("want the exact topic as the first demand frame, got %+v", f)
	}
}

func TestDemandDeduplicatesRepeatSubscribes(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	provider.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})

	sub := dialClient(t, url)
	sub.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("subscribed")
	f := provider.readDemand()
	if f.Topic != "agent.conversation.s1" || !f.Demand {
		t.Fatalf("got %+v", f)
	}
	// An idempotent client re-subscribe must not inflate the count — otherwise
	// the later unsubscribe leaves count=1 forever and the stream never stops.
	sub.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("subscribed")
	sub.send(Frame{Op: "unsubscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("unsubscribed")
	f = provider.readDemand()
	if f.Topic != "agent.conversation.s1" || f.Demand {
		t.Fatalf("want release after the single real subscriber left, got %+v", f)
	}
}

func TestDemandWatchReplaysExistingDemand(t *testing.T) {
	url := rpcServer(t)

	// Demand exists BEFORE the provider watches — the restarted-brain case.
	// Without the replay this session stays silently dead until the client
	// happens to re-subscribe.
	sub := dialClient(t, url)
	sub.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("subscribed")

	provider := dialClient(t, url)
	provider.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})
	f := provider.readDemand()
	if f.Topic != "agent.conversation.s1" || !f.Demand {
		t.Fatalf("watch did not replay pre-existing demand: %+v", f)
	}
}

func TestDemandClearWatchStopsFrames(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	provider.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})
	// topics:[] clears the watch.
	provider.send(Frame{Op: "demand", Topics: nil})

	sub := dialClient(t, url)
	sub.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("subscribed")

	// Demand was still COUNTED while unwatched — re-arming the watch replays it.
	provider.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})
	f := provider.readDemand()
	if f.Topic != "agent.conversation.s1" || !f.Demand {
		t.Fatalf("re-armed watch did not replay demand counted while cleared: %+v", f)
	}
	// Exactly ONE frame for s1 arrived (the replay). Had the cleared watch
	// leaked the original subscribe's transition too, a stale duplicate
	// demand:true would be queued ahead of this release.
	sub.send(Frame{Op: "unsubscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("unsubscribed")
	f = provider.readDemand()
	if f.Topic != "agent.conversation.s1" || f.Demand {
		t.Fatalf("want the release next — a cleared watch delivered a frame anyway: %+v", f)
	}
}

// demandEntitlementServer is providerTokenServer plus a view-tier token, so the
// two entitlement rules can be tested against the real tiers rather than
// fabricated conns.
func demandEntitlementServer(t *testing.T) (url string, providerTok, viewTok string) {
	t.Helper()
	file := filepath.Join(t.TempDir(), "tokens.json")
	prov, err := authtoken.Mint(file, authtoken.ScopeProvider, "node")
	if err != nil {
		t.Fatal(err)
	}
	view, err := authtoken.Mint(file, authtoken.ScopeView, "phone")
	if err != nil {
		t.Fatal(err)
	}
	store := authtoken.NewStore(file)
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		r, ok := store.Lookup(tok)
		if !ok {
			return ScopedIdent{}, false
		}
		return ScopedIdent{
			Scope:    string(r.Scope),
			Methods:  r.Scope.Methods(),
			Provides: r.ProvidesGrant(),
		}, true
	})
	return url, prov.Token, view.Token
}

// Only a subscriber ENTITLED to consume the topic creates demand. A
// provider-tier connection may not consume fleet topics at all (mayConsume is
// hard-false for that tier), so its subscribe must not make another provider
// produce the stream — otherwise a credential the hub would refuse delivery to
// can still cause all the work.
func TestDemandNotCreatedByUnentitledSubscriber(t *testing.T) {
	url, providerTok, viewTok := demandEntitlementServer(t)

	watcher := dialClientToken(t, url, providerTok)
	watcher.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})

	unentitled := dialClientToken(t, url, providerTok)
	unentitled.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	unentitled.readUntil("subscribed")

	// A view token holds sessions.conversation, which guards the topic — its
	// subscribe is real demand, and this also pins that a scoped USER token
	// (not just the trusted host) can turn the feed on. It subscribes to a
	// DIFFERENT session first: if the unentitled subscribe above had counted,
	// its s1 frame would arrive ahead of this s2 one.
	entitled := dialClientToken(t, url, viewTok)
	entitled.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s2"}})
	entitled.readUntil("subscribed")
	f := watcher.readDemand()
	if f.Topic != "agent.conversation.s2" || !f.Demand {
		t.Fatalf("want s2 as the first demand frame (s1's subscriber may not consume it), got %+v", f)
	}
}

// Watching demand is gated on mayPublish: you may learn a stream is wanted
// exactly when you are allowed to produce it. A view token cannot publish
// agent.conversation.* (its Provides is nil), so its watch hears nothing —
// "which sessions is somebody reading" is itself information.
func TestDemandWatchGatedOnMayPublish(t *testing.T) {
	url, providerTok, viewTok := demandEntitlementServer(t)

	snoop := dialClientToken(t, url, viewTok)
	snoop.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})

	producer := dialClientToken(t, url, providerTok)
	producer.send(Frame{Op: "demand", Topics: []string{"agent.conversation."}})

	sub := dialClientToken(t, url, viewTok)
	sub.send(Frame{Op: "subscribe", Topics: []string{"agent.conversation.s1"}})
	sub.readUntil("subscribed")

	// The provider-tier watcher (NOT trusted — this is the remote-node
	// deployment the feature exists for) hears the transition…
	f := producer.readDemand()
	if f.Topic != "agent.conversation.s1" || !f.Demand {
		t.Fatalf("provider-tier watcher did not hear demand: %+v", f)
	}
	// …and the view-tier watcher hears nothing, ever.
	snoop.expectNoDemand()
}
