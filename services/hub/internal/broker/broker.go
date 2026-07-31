// Package broker is the in-memory pub/sub core of the hub. Fan-out is
// non-blocking by design: a subscriber that can't keep up has events dropped
// (and counted) rather than stalling the publisher or other subscribers — the
// bus must never stutter because one client is slow.
package broker

import (
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

const defaultBuffer = 64

// StreamTopicPrefix marks event types carrying a byte STREAM rather than a
// discrete fact. Dropping a discrete event costs the consumer one fact;
// dropping a chunk of a byte stream silently corrupts everything the consumer
// reconstructs from it — a terminal grid renders garbled escape sequences with
// no indication anything was lost. So drops on these topics are remembered and
// reported (see [Subscription.TakeDesyncs]) instead of only counted.
const StreamTopicPrefix = "pty.bytes."

// Cap on remembered desynced topics, so a client that is down for a long time
// can't accumulate one entry per session it ever watched.
const maxDesyncTopics = 64

// MaxTopics caps how many patterns one subscription retains, for the same
// reason maxDesyncTopics caps the desync map: the topic list is walked on every
// publish (see [Subscription.matches]) under a lock the fan-out holds, so an
// unbounded list turns one client's `subscribe` into a stall for every
// publisher on the bus. Past the cap further patterns are simply not added —
// the bus layer rejects oversized subscribe frames outright, so hitting this is
// already pathological. Real clients subscribe to a handful of patterns or "*".
const MaxTopics = 512

// Subscription is a live consumer. Events matching its topics arrive on C.
type Subscription struct {
	C chan event.Envelope

	id       uint64
	mu       sync.RWMutex
	topics   []string
	topicSet map[string]struct{} // membership index for topics; guarded by mu
	dropped  atomic.Uint64
	// Stream topics that lost at least one event since the consumer last
	// checked. Guarded by mu.
	desynced map[string]struct{}
}

// Dropped returns how many events were discarded because C was full.
func (s *Subscription) Dropped() uint64 { return s.dropped.Load() }

// noteDrop records a discarded event. Stream topics are remembered by name so
// the consumer can be told to resync; everything else is only counted.
func (s *Subscription) noteDrop(typ string) {
	s.dropped.Add(1)
	if !strings.HasPrefix(typ, StreamTopicPrefix) {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.desynced == nil {
		s.desynced = make(map[string]struct{})
	}
	if _, ok := s.desynced[typ]; !ok && len(s.desynced) >= maxDesyncTopics {
		return
	}
	s.desynced[typ] = struct{}{}
}

// TakeDesyncs returns and clears the stream topics that have lost data since
// the last call. A consumer of a byte stream cannot recover the missing bytes
// and must re-attach to get a fresh replay; this is how it finds out it needs
// to. Nil when nothing was dropped, which is the overwhelmingly common case.
func (s *Subscription) TakeDesyncs() []string {
	s.mu.RLock()
	empty := len(s.desynced) == 0
	s.mu.RUnlock()
	if empty {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]string, 0, len(s.desynced))
	for t := range s.desynced {
		out = append(out, t)
	}
	s.desynced = nil
	return out
}

// SetTopics replaces the subscription's topic patterns.
func (s *Subscription) SetTopics(topics []string) {
	s.mu.Lock()
	s.topics, s.topicSet = nil, nil
	s.addLocked(topics)
	s.mu.Unlock()
}

// AddTopics adds patterns to the subscription, de-duplicating.
func (s *Subscription) AddTopics(topics ...string) {
	s.mu.Lock()
	s.addLocked(topics)
	s.mu.Unlock()
}

// addLocked appends the patterns not already held, up to MaxTopics. De-dup goes
// through topicSet rather than a scan of s.topics: the scan made applying N
// patterns cost O(N²) string comparisons *while holding the write lock* that
// every publisher's fan-out needs, so a single client could stall the whole bus
// for seconds with one large subscribe frame.
func (s *Subscription) addLocked(topics []string) {
	for _, t := range topics {
		if len(s.topics) >= MaxTopics {
			return
		}
		if _, dup := s.topicSet[t]; dup {
			continue
		}
		if s.topicSet == nil {
			s.topicSet = make(map[string]struct{}, len(topics))
		}
		s.topicSet[t] = struct{}{}
		s.topics = append(s.topics, t)
	}
}

// RemoveTopics drops the given patterns from the subscription.
func (s *Subscription) RemoveTopics(topics ...string) {
	// Index the removals first so the filter below is one pass, not one scan of
	// the argument per retained topic — RemoveTopics is the same
	// quadratic-under-lock primitive AddTopics was.
	drop := make(map[string]struct{}, len(topics))
	for _, t := range topics {
		drop[t] = struct{}{}
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	kept := s.topics[:0]
	for _, t := range s.topics {
		if _, gone := drop[t]; gone {
			delete(s.topicSet, t)
			continue
		}
		kept = append(kept, t)
	}
	s.topics = kept
}

// Topics returns a copy of the current patterns.
func (s *Subscription) Topics() []string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return append([]string(nil), s.topics...)
}

func (s *Subscription) matches(typ string) bool {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return event.MatchesAny(s.topics, typ)
}

// Broker fans events out to matching subscribers.
type Broker struct {
	mu     sync.RWMutex
	subs   map[uint64]*Subscription
	nextID uint64
	buffer int

	now func() time.Time
	seq atomic.Uint64
}

// New returns a broker with the default per-subscriber buffer.
func New() *Broker { return NewWithBuffer(defaultBuffer) }

// NewWithBuffer returns a broker whose subscribers each get a channel of the
// given capacity. Mainly useful for tests that exercise the drop path.
func NewWithBuffer(buffer int) *Broker {
	if buffer < 1 {
		buffer = 1
	}
	return &Broker{subs: make(map[uint64]*Subscription), buffer: buffer, now: time.Now}
}

// Subscribe registers a consumer for the given topic patterns (may be nil and
// added later via AddTopics).
func (b *Broker) Subscribe(topics []string) *Subscription {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.nextID++
	sub := &Subscription{
		C:  make(chan event.Envelope, b.buffer),
		id: b.nextID,
	}
	sub.addLocked(topics) // not yet published, so no other goroutine can see it
	b.subs[sub.id] = sub
	return sub
}

// Unsubscribe removes a consumer and closes its channel. Safe to call once.
func (b *Broker) Unsubscribe(sub *Subscription) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if _, ok := b.subs[sub.id]; ok {
		delete(b.subs, sub.id)
		close(sub.C)
	}
}

// Publish stamps ev (id/time if blank) then delivers it to every matching
// subscriber without blocking. This is the single stamping point, so events
// from internal producers (supervisor, bridges) and from WS clients are treated
// identically.
func (b *Broker) Publish(ev event.Envelope) {
	if ev.ID == "" {
		ev.ID = "ev-" + strconv.FormatUint(b.seq.Add(1), 10)
	}
	if ev.Time.IsZero() {
		ev.Time = b.now()
	}
	b.mu.RLock()
	defer b.mu.RUnlock()
	for _, sub := range b.subs {
		if !sub.matches(ev.Type) {
			continue
		}
		select {
		case sub.C <- ev:
		default:
			sub.noteDrop(ev.Type)
		}
	}
}

// SubscriberCount returns the number of active subscriptions.
func (b *Broker) SubscriberCount() int {
	b.mu.RLock()
	defer b.mu.RUnlock()
	return len(b.subs)
}
