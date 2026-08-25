package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

// RE-REGISTRATION MUST SURVIVE A STALE PREDECESSOR.
//
// The hub's registration guard is first-registration-wins, and a slot is only
// released when the owning connection's read loop returns. A node that stops
// without severing its TCP connection cleanly therefore leaves a dead provider
// holding every capability it registered — and when that same node wakes and
// its brain dials back in, the hub REFUSES the whole registration. The brain
// sends `register` exactly once per connect and never reads the ack, so it
// would sit there believing it had registered, providing nothing, until
// something else made it reconnect.
//
// The hub's own eviction (bus.Server.EvictConn, driven by the node
// supervisor's liveness poll) is the primary fix. This is the second half:
// the brain reads its OWN ack and keeps asking while the hub is withholding
// methods it asked for, so a wake recovers even if the eviction has not
// happened yet — or never happens at all, because the hub is an older build.
func TestBrainReRegistersWhenTheHubWithheldMethods(t *testing.T) {
	prev := registerRetryInterval
	registerRetryInterval = 100 * time.Millisecond
	t.Cleanup(func() { registerRetryInterval = prev })

	srv := bus.NewServer(broker.New())
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	wsURL := strings.Replace(hs.URL, "http://", "ws://", 1) + "/bus"

	// The zombie: a provider connection that owns brain.info and will never
	// answer anything. From the hub's side it is indistinguishable from a
	// healthy idle provider until somebody decides otherwise.
	dialCtx, cancelDial := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancelDial()
	zombie, _, err := websocket.Dial(dialCtx, wsURL, nil)
	if err != nil {
		t.Fatalf("dial zombie: %v", err)
	}
	reg, _ := json.Marshal(map[string]any{"op": "register", "methods": []string{"brain.info"}})
	if err := zombie.Write(dialCtx, websocket.MessageText, reg); err != nil {
		t.Fatalf("zombie register: %v", err)
	}
	// Wait until the hub has actually granted the zombie the slot, so the brain
	// below is genuinely refused rather than racing it.
	deadline := time.Now().Add(3 * time.Second)
	for {
		if _, ok := srv.ProviderConnID("brain.info"); ok {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("zombie never took the brain.info slot")
		}
		time.Sleep(10 * time.Millisecond)
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	b := newBusClient(wsURL, "", []string{"brain.info"},
		func(context.Context, string, json.RawMessage) (json.RawMessage, error) {
			return json.RawMessage(`{"provider":"brain"}`), nil
		})
	go b.run(ctx)

	// Give the brain time to connect and be refused, then evict the zombie the
	// way the node supervisor's liveness poll would.
	time.Sleep(300 * time.Millisecond)
	id, ok := srv.ProviderConnID("brain.info")
	if !ok {
		t.Fatal("nobody owns brain.info after the brain connected")
	}
	srv.EvictConn(id)

	// The brain must reclaim the slot ON ITS OWN — no reconnect, no restart.
	deadline = time.Now().Add(5 * time.Second)
	for {
		if _, ok := srv.ProviderConnID("brain.info"); ok {
			return // reclaimed
		}
		if time.Now().After(deadline) {
			t.Fatal("the brain never re-registered brain.info after the stale owner was evicted — a woken node would provide nothing")
		}
		time.Sleep(25 * time.Millisecond)
	}
}
