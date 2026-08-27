package main

// Conversation deltas over the bus — the transcript feed, pushed instead of
// polled, and only for sessions somebody has open.
//
// THE PROBLEM THIS REPLACES. A sparse brain row carries no conversation by
// design (parity_test.go: folding a transcript into every state tick would ship
// whole transcripts per state change), so a web client fetches
// sessions.conversation itself. While a turn streams it has to fetch on a
// clock, and claudemon COALESCES a streaming reply into one item that grows in
// place — so every fetch re-downloads the whole in-progress message. Measured
// on a 2.6 KB reply: 46 fetches, 62 KB. The cost is quadratic in reply length,
// and no anchor tuning gets off that curve because the unit of the answer is
// the item, not the fragment.
//
// claudemon already publishes the fragments: `GET /conversation/stream` is a
// fleet-wide SSE of ConversationDelta{session_id, seq, reset, items} whose
// items are the raw pieces (ConversationStore::push broadcasts what it was
// handed; only the retained log folds). The desktop and the TUI consume it over
// loopback. This forwards it onto the bus for everyone else.
//
// DEMAND-GATED, because the sparse-row rationale has to survive. The hub tells
// us which `agent.conversation.<id>` topics have a subscriber
// (internal/bus/demand.go) and we forward exactly those. No open pane, no
// bytes. A dropped websocket releases its demand at the hub the instant the
// socket dies, so there is no lease TTL of firehose to pay for.
//
// ONE STREAM, NOT ONE PER SESSION OR PER CLIENT. claudemon's endpoint is
// fleet-wide, so N watched sessions across M clients cost one SSE connection
// here and one bus event per fragment, fanned out by the broker.

import (
	"context"
	"encoding/json"
	"log"
	"sync"
	"time"
)

// conversationTopicPrefix is the family this forwarder publishes and the prefix
// it asks the hub for demand on. The concrete topic is prefix + sessionId.
// Classified in capspec/eventtopics.go as guarded by sessions.conversation.
const conversationTopicPrefix = "agent.conversation."

// conversationHub forwards claudemon's conversation deltas for the sessions the
// hub reports demand for.
type conversationHub struct {
	cm      *claudemonClient
	publish func(eventType string, data json.RawMessage)
	// visible is the fleet-visibility rule, by session id. A session the shared
	// layout hides is refused here for the same reason its snapshot and its
	// status line are: a transcript is strictly more disclosure than the row it
	// belongs to. Nil fails CLOSED.
	visible func(sessionID string) bool

	mu     sync.Mutex
	wanted map[string]bool
	// cancel stops the live SSE consumer; nil when none is running (nothing is
	// wanted). The consumer is started on the first demand and stopped on the
	// last release, so an idle fleet holds no connection to claudemon at all.
	cancel context.CancelFunc
	parent context.Context
}

func newConversationHub(ctx context.Context, cm *claudemonClient, publish func(string, json.RawMessage), visible func(string) bool) *conversationHub {
	return &conversationHub{cm: cm, publish: publish, visible: visible, parent: ctx, wanted: map[string]bool{}}
}

// setDemand starts or stops forwarding one session's deltas. Idempotent in both
// directions: the hub only announces 0↔1 transitions, but a provider that
// reconnects gets the whole current demand set replayed, so repeats are normal.
func (h *conversationHub) setDemand(sessionID string, on bool) {
	if sessionID == "" {
		return
	}
	h.mu.Lock()
	if !on {
		delete(h.wanted, sessionID)
		if len(h.wanted) == 0 && h.cancel != nil {
			h.cancel()
			h.cancel = nil
		}
		h.mu.Unlock()
		return
	}
	fresh := !h.wanted[sessionID]
	h.wanted[sessionID] = true
	start := h.cancel == nil
	if start {
		ctx, cancel := context.WithCancel(h.parent)
		h.cancel = cancel
		go h.run(ctx)
	}
	h.mu.Unlock()
	if fresh {
		h.announceReady(sessionID)
	}
}

// announceReady publishes the handshake frame: proof, to the client, that the
// whole push path exists on this hub AND on this node.
//
// Feature detection has to be end-to-end here, and a version flag cannot do it.
// A new client against an old hub (no demand table) and a new client against an
// old node (no forwarder) fail identically — the subscribe succeeds and nothing
// ever arrives — so the client cannot tell "push is live, the session is idle"
// from "push does not exist here" without a positive signal. This frame IS the
// signal: it travelled the demand table, the provider watch, the topic
// classification and the delivery filter to get here. Until it lands, the
// client keeps its poll tick running.
func (h *conversationHub) announceReady(sessionID string) {
	if h.visible == nil || !h.visible(sessionID) {
		return
	}
	payload, err := json.Marshal(map[string]any{"session_id": sessionID, "ready": true})
	if err != nil {
		return
	}
	// The literal prefix (rather than conversationTopicPrefix) is what lets
	// capspec's publish-site scanner see this as a topic FAMILY — an identifier
	// resolves to an exact topic, and "agent.conversation." exact is a dead row.
	h.publish("agent.conversation."+sessionID, payload)
}

// reset forgets all demand and stops the consumer. Called once per bus
// connection, before the watch is re-armed: see busClient.resetDemand.
func (h *conversationHub) reset() {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.wanted = map[string]bool{}
	if h.cancel != nil {
		h.cancel()
		h.cancel = nil
	}
}

// run follows claudemon's fleet-wide delta stream until ctx is cancelled (the
// last watcher went away) or the brain shuts down, reconnecting with backoff.
func (h *conversationHub) run(ctx context.Context) {
	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		start := time.Now()
		err := h.cm.streamSSE(ctx, "/conversation/stream", func(name string, data []byte) {
			if name != "conversation.delta" && name != "" {
				return
			}
			h.forward(data)
		})
		if ctx.Err() != nil {
			return
		}
		backoff = backoffAfterConn(backoff, time.Since(start))
		if err != nil {
			// Same reasoning as the /events consumer: swallowed, this is a
			// transcript that silently stops updating on every client at once,
			// with nothing anywhere saying why.
			log.Printf("brain: claudemon /conversation/stream ended: %v (retry in %s)", err, backoff)
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(backoff):
		}
		if backoff < 10*time.Second {
			backoff *= 2
		}
	}
}

// forward relays one delta VERBATIM to its session's topic, if that session is
// both wanted and visible. Verbatim because the client's fold is written
// against claudemon's own item vocabulary — the same shapes the desktop bridge
// and the TUI consume — and a re-shaping here would be a third dialect to keep
// in step.
func (h *conversationHub) forward(data []byte) {
	var d struct {
		SessionID string `json:"session_id"`
	}
	if json.Unmarshal(data, &d) != nil || d.SessionID == "" {
		return
	}
	h.mu.Lock()
	want := h.wanted[d.SessionID]
	h.mu.Unlock()
	if !want {
		return
	}
	if h.visible == nil || !h.visible(d.SessionID) {
		return
	}
	h.publish("agent.conversation."+d.SessionID, json.RawMessage(append([]byte(nil), data...)))
}

// visibleSessionRule adapts the fleet-visibility rule to a by-id question, the
// shape a delta arrives in. Same composition as visibleStatusLinePublisher, and
// the same two independent checks: the store must ADMIT to having the session
// (an unknown id is not "visible by default"), and the rule must pass it.
func visibleSessionRule(store *sessionStore, vis fleetVisibility) func(string) bool {
	return func(id string) bool {
		if vis == nil || store == nil {
			return false
		}
		snap, ok := store.get(id)
		if !ok {
			return false
		}
		return vis.visible(context.Background(), snap)
	}
}
