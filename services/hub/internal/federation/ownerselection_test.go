package federation

// A federated snapshot is republished with its payload bytes untouched, and the
// daemon-owned model facts are the case that makes that property load-bearing:
// `requestedSelection` (what was asked for), `resolvedContextWindow` (what the
// owner resolved) and the provider's own contradicted `contextWindowSize` are
// three separate claims a client reconciles itself. A link that re-marshaled,
// completed or dropped any of them would hand the client a different question
// than the one its home hub asked.

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

func TestFederatedSnapshotCarriesOwnerSelectionVerbatim(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()

	peerSrv := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer peerSrv.Close()
	peerURL := strings.Replace(peerSrv.URL, "http", "ws", 1) + "/bus"

	localBroker := broker.New()
	localBus := bus.NewServer(localBroker)
	fed, err := New(localBroker, []Peer{{Name: "work", URL: peerURL}})
	if err != nil {
		t.Fatal(err)
	}
	localBus.SetFederation(fed)
	go fed.Run(ctx)
	localSrv := httptest.NewServer(localBus.Handler())
	defer localSrv.Close()
	localURL := strings.Replace(localSrv.URL, "http", "ws", 1) + "/bus"

	events := make(chan event.Envelope, 32)
	sub := dial(t, ctx, localURL)
	defer sub.CloseNow()
	send(t, ctx, sub, wsFrame{Op: "subscribe", Topics: []string{"agent.*", "hub.peer.*"}})
	go func() {
		for {
			_, data, err := sub.Read(ctx)
			if err != nil {
				return
			}
			var f wsFrame
			if json.Unmarshal(data, &f) == nil && f.Op == "event" && f.Event != nil {
				events <- *f.Event
			}
		}
	}()

	next := func() event.Envelope {
		t.Helper()
		select {
		case ev := <-events:
			return ev
		case <-time.After(8 * time.Second):
			t.Fatal("no event arrived on the local bus")
			return event.Envelope{}
		}
	}
	if ev := next(); ev.Type != "hub.peer.connected" {
		t.Fatalf("expected hub.peer.connected first, got %+v", ev)
	}

	// Two rows in one payload: an early 1M session whose provider still claims
	// 200,000, and a session whose selection is sparse (identity, no window)
	// with no resolved window at all.
	const payload = `{"sessionId":"remote-1","sparse":true,` +
		`"requestedSelection":{"model":"opus","contextWindow":1000000},` +
		`"resolvedContextWindow":1000000,` +
		`"usage":{"contextTokens":356380,"contextLimit":1000000},` +
		`"statusLine":{"contextWindowSize":200000,"contextUsedPct":178.19},` +
		`"peer":{"sessionId":"remote-2","requestedSelection":{"model":"sonnet","contextWindow":null}}}`

	peerPub := dial(t, ctx, peerURL)
	defer peerPub.CloseNow()
	send(t, ctx, peerPub, wsFrame{Op: "publish", Event: &event.Envelope{
		Type: "agent.snapshot", Source: "test", Data: json.RawMessage(payload),
	}})

	ev := next()
	if ev.Type != "agent.snapshot" || ev.Hub != "work" {
		t.Fatalf("federated event wrong: type=%q hub=%q", ev.Type, ev.Hub)
	}
	if string(ev.Data) != payload {
		t.Fatalf("payload rewritten across the link:\n got %s\nwant %s", ev.Data, payload)
	}
}
