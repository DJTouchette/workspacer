package bus

// DEMAND SIGNALLING — "is anybody actually listening to this topic?", answered
// to the provider that would have to produce it.
//
// Some feeds are expensive enough that publishing them unconditionally is a
// design error rather than an inefficiency. The headless brain's session rows
// are deliberately SPARSE — no transcript — because folding one into every
// state tick would ship megabytes per second across a fleet (cmd/brain's
// parity record says so in as many words). The conversation delta feed has the
// same shape: cheap for the two sessions somebody is watching, ruinous for the
// forty nobody is.
//
// The lease pattern already in this tree (sessions.attachTerminal +
// terminalKeepalive, swept on a TTL) answers that question with a timer, which
// costs a TTL of firehose after a client vanishes and cannot tell three
// watchers apart from one. But the bus ALREADY tracks the exact fact a provider
// needs — which connections subscribed to which topics — and already releases
// it when a socket dies (handleBus's `defer s.broker.Unsubscribe(sub)`). This
// file makes that bookkeeping legible to providers instead of building a
// parallel lease plane beside it:
//
//	provider -> hub  {op:"demand", topics:["agent.conversation."]}
//	hub -> provider  {op:"demand", topic:"agent.conversation.<id>", demand:true}
//	hub -> provider  {op:"demand", topic:"agent.conversation.<id>"}   // released
//
// Only 0↔1 transitions are announced, so N clients on one session cost the
// provider one start and one stop.
//
// TWO RULES KEEP IT HONEST.
//
// Only EXACT topics create demand. The desktop subscribes to "*"; if a wildcard
// counted, every session on the bus would acquire a transcript firehose the
// moment one client connected, which is precisely the bandwidth the sparse-row
// design exists to avoid. A wildcard subscriber still RECEIVES what somebody
// else demanded — it simply never causes it.
//
// Only a subscriber that may CONSUME the topic counts. Otherwise a credential
// the hub would refuse delivery to could still make a provider do the work,
// which is a denial-of-service dressed as a subscribe frame.
//
// And watching demand is gated on mayPublish, not on `trusted`: a remote node's
// `brain --hub` is provider-tier and NOT trusted, so a trust check would make
// this useless on exactly the deployment it was built for. "You may learn that
// a stream is wanted when you are allowed to produce it" is the same rule
// mayPublish already applies to producing it.

import (
	"strings"
	"sync"
)

// maxDemandTopics bounds the counts table. It is already bounded in practice by
// (live connections × broker.MaxTopics), but a table walked on every publish-
// side transition should not be able to grow without a stated ceiling. Past it
// new topics simply go uncounted — which fails CLOSED (no demand, no bytes).
const maxDemandTopics = 4096

// demandWatcher is one provider connection listening for demand transitions
// under a set of topic prefixes.
type demandWatcher struct {
	cn       *conn
	prefixes []string
}

// demandTable counts exact-topic subscriptions and announces 0↔1 transitions to
// watching providers.
type demandTable struct {
	mu       sync.Mutex
	counts   map[string]int             // exact topic -> subscribers
	held     map[uint64]map[string]bool // conn id -> the topics it is counted for
	watchers map[uint64]*demandWatcher
}

func newDemandTable() *demandTable {
	return &demandTable{
		counts:   map[string]int{},
		held:     map[uint64]map[string]bool{},
		watchers: map[uint64]*demandWatcher{},
	}
}

// countable reports whether a subscribe frame's topic is one this table tracks:
// a concrete topic (no wildcard) that this connection is actually allowed to
// receive.
func countable(cn *conn, topic string) bool {
	if topic == "" || strings.Contains(topic, "*") {
		return false
	}
	return cn.mayConsume(topic)
}

// add counts a connection's new subscriptions, returning the notifications the
// caller must deliver. De-duplicated per connection: subscribing twice to one
// topic is one subscriber, so an idempotent client re-subscribe cannot inflate
// the count and strand a stream nobody is watching.
func (d *demandTable) add(cn *conn, topics []string) []func() {
	if d == nil || cn == nil {
		return nil
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	var risen []string
	for _, t := range topics {
		if !countable(cn, t) {
			continue
		}
		held := d.held[cn.id]
		if held == nil {
			held = map[string]bool{}
			d.held[cn.id] = held
		}
		if held[t] {
			continue
		}
		// The ceiling applies to NEW topics only: a second subscriber to a
		// topic already counted must always be counted too, or its departure
		// would announce demand:false while the other is still subscribed.
		if _, known := d.counts[t]; !known && len(d.counts) >= maxDemandTopics {
			continue
		}
		held[t] = true
		d.counts[t]++
		if d.counts[t] == 1 {
			risen = append(risen, t)
		}
	}
	return d.notifyLocked(risen, true)
}

// remove drops a connection's subscriptions to the given topics.
func (d *demandTable) remove(cn *conn, topics []string) []func() {
	if d == nil || cn == nil {
		return nil
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.notifyLocked(d.dropLocked(cn.id, topics), false)
}

// release drops everything a connection held — its subscriptions AND its
// watch. Called from handleBus's defer, so a dropped websocket cannot leave a
// firehose running: the socket dying IS the unsubscribe.
func (d *demandTable) release(cn *conn) []func() {
	if d == nil || cn == nil {
		return nil
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	delete(d.watchers, cn.id)
	held := d.held[cn.id]
	all := make([]string, 0, len(held))
	for t := range held {
		all = append(all, t)
	}
	return d.notifyLocked(d.dropLocked(cn.id, all), false)
}

// dropLocked removes topics from a connection's holdings and returns the ones
// whose count fell to zero.
func (d *demandTable) dropLocked(id uint64, topics []string) []string {
	held := d.held[id]
	if held == nil {
		return nil
	}
	var fallen []string
	for _, t := range topics {
		if !held[t] {
			continue
		}
		delete(held, t)
		if d.counts[t] <= 1 {
			delete(d.counts, t)
			fallen = append(fallen, t)
			continue
		}
		d.counts[t]--
	}
	if len(held) == 0 {
		delete(d.held, id)
	}
	return fallen
}

// watch registers (or, with no prefixes, clears) a provider's interest and
// returns the notifications to deliver: the transition frames it will receive
// from now on, preceded by a REPLAY of the demand that already exists.
//
// The replay is load-bearing, not a nicety. Demand transitions 0→1 exactly
// once; a provider that restarts while a client is already subscribed would
// otherwise never hear about it and the feed would stay silently dead until
// that client happened to re-subscribe.
func (d *demandTable) watch(cn *conn, prefixes []string) []func() {
	if d == nil || cn == nil {
		return nil
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	if len(prefixes) == 0 {
		delete(d.watchers, cn.id)
		return nil
	}
	w := &demandWatcher{cn: cn, prefixes: append([]string(nil), prefixes...)}
	d.watchers[cn.id] = w
	var out []func()
	for topic := range d.counts {
		if !w.covers(topic) || !cn.mayPublish(topic) {
			continue
		}
		out = append(out, sendDemand(cn, topic, true))
	}
	return out
}

// notifyLocked builds one send per watcher entitled to hear about each topic.
// Sends are returned as thunks rather than performed here: a websocket write
// under this mutex would let one slow provider stall every subscribe frame on
// the bus.
func (d *demandTable) notifyLocked(topics []string, wanted bool) []func() {
	if len(topics) == 0 || len(d.watchers) == 0 {
		return nil
	}
	var out []func()
	for _, w := range d.watchers {
		for _, topic := range topics {
			if !w.covers(topic) || !w.cn.mayPublish(topic) {
				continue
			}
			out = append(out, sendDemand(w.cn, topic, wanted))
		}
	}
	return out
}

func sendDemand(cn *conn, topic string, wanted bool) func() {
	return func() { _ = cn.send(Frame{Op: "demand", Topic: topic, Demand: wanted}) }
}

// covers reports whether a topic falls under any of the watcher's prefixes. A
// prefix is a literal string match on the front of the topic — the watcher asks
// for "agent.conversation." and hears about "agent.conversation.<id>".
func (w *demandWatcher) covers(topic string) bool {
	for _, p := range w.prefixes {
		if p != "" && strings.HasPrefix(topic, p) {
			return true
		}
	}
	return false
}

// deliver runs the notifications a table call handed back, off the lock.
func deliver(sends []func()) {
	for _, s := range sends {
		s()
	}
}

// demandCount reports how many subscribers a topic has. Test/introspection
// helper — the count is otherwise invisible from outside the package.
func (d *demandTable) demandCount(topic string) int {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.counts[topic]
}
