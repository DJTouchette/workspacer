package main

// A live, in-memory session store. In full scope the brain consumes claudemon's
// /events stream (see events.go) and keeps the latest snapshot per session here,
// so agents.list / sessions.snapshot* answer from memory and every change is
// pushed to the bus as an `agent.snapshot` event — the foundation of the
// streaming phase (clients render live instead of polling). This is also where
// name/parent enrichment will layer in.

import (
	"encoding/json"
	"sort"
	"sync"
)

type sessionStore struct {
	mu sync.RWMutex
	m  map[string]json.RawMessage // session_id -> snapshot JSON (claudemon's shape)

	// onChange is invoked (outside the lock) after a set, to publish the update.
	onChange func(id string, snap json.RawMessage)
	// enrich, if set, overlays name/parent/etc. onto each snapshot as it lands.
	enrich func(json.RawMessage) json.RawMessage
	// onSeed is invoked (outside the lock) with the whole seeded set, which
	// onChange deliberately never sees. The finish watcher needs it: a
	// transition has two halves, and without the BEFORE state of every session
	// that already existed at boot, the first finish after startup looks like a
	// first sighting and wakes nobody. See finishWatcher.prime.
	onSeed func(map[string]json.RawMessage)
}

func (s *sessionStore) applyEnrich(snap json.RawMessage) json.RawMessage {
	if s.enrich == nil {
		return snap
	}
	return s.enrich(snap)
}

func newSessionStore() *sessionStore {
	return &sessionStore{m: map[string]json.RawMessage{}}
}

// seed replaces the whole store without firing onChange — used for the initial
// snapshot so we don't publish a burst of events for pre-existing sessions.
func (s *sessionStore) seed(snaps map[string]json.RawMessage) {
	enriched := make(map[string]json.RawMessage, len(snaps))
	for id, snap := range snaps {
		enriched[id] = s.applyEnrich(snap)
	}
	s.mu.Lock()
	s.m = enriched
	cb := s.onSeed
	s.mu.Unlock()
	if cb != nil {
		cb(enriched)
	}
}

// set upserts one session and notifies (publishes) the change.
func (s *sessionStore) set(id string, snap json.RawMessage) {
	snap = s.applyEnrich(snap)
	s.mu.Lock()
	s.m[id] = snap
	cb := s.onChange
	s.mu.Unlock()
	if cb != nil {
		cb(id, snap)
	}
}

// updateStatusLine merges a fresh status_line into a known session's snapshot,
// silently (no onChange) — statusline ticks are high-frequency, so they update
// the store for polls/next-snapshot but are pushed on the lighter
// `agent.statusline` event, not by re-publishing the whole snapshot. Unknown
// sessions are skipped (nothing to merge into yet).
func (s *sessionStore) updateStatusLine(id string, statusLine json.RawMessage) {
	s.mu.Lock()
	defer s.mu.Unlock()
	snap, ok := s.m[id]
	if !ok {
		return
	}
	var m map[string]any
	if json.Unmarshal(snap, &m) != nil {
		return
	}
	var sl any
	if json.Unmarshal(statusLine, &sl) != nil {
		return
	}
	m["status_line"] = sl
	if out, err := json.Marshal(m); err == nil {
		s.m[id] = out
	}
}

func (s *sessionStore) get(id string) (json.RawMessage, bool) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	snap, ok := s.m[id]
	return snap, ok
}

// all returns every snapshot, ordered by session id for deterministic output.
func (s *sessionStore) all() []json.RawMessage {
	s.mu.RLock()
	ids := make([]string, 0, len(s.m))
	for id := range s.m {
		ids = append(ids, id)
	}
	out := make([]json.RawMessage, 0, len(s.m))
	sort.Strings(ids)
	for _, id := range ids {
		out = append(out, s.m[id])
	}
	s.mu.RUnlock()
	return out
}

// snapshotID extracts the session id from a claudemon session snapshot.
func snapshotID(snap json.RawMessage) string {
	var x struct {
		SessionID string `json:"session_id"`
	}
	_ = json.Unmarshal(snap, &x)
	return x.SessionID
}

// remove forgets one session. The ONE caller is agents.close, and it exists
// because claudemon does not forget: the daemon keeps a stopped session as a
// resumable row on purpose, so a dismissal that only signalled would leave
// agents.list unchanged and the verb would do nothing a caller could see.
//
// Silent (no onChange): the row is gone, and publishing an agent.snapshot for a
// session that no longer exists would ask every client to render it again.
func (s *sessionStore) remove(id string) {
	s.mu.Lock()
	delete(s.m, id)
	s.mu.Unlock()
}

// restamp re-runs enrichment over a row already in the store, in place.
//
// The overlay (label / parentSessionId / isWakeTarget) is applied when a
// snapshot LANDS, so a change to the spawn metadata behind it — the one
// agents.reparent makes — is invisible until claudemon next pushes that
// session. A manager that adopts a fleet and immediately lists it would see the
// state before its own move. This closes that window without inventing a second
// source of truth: it re-enriches the row the store already holds.
//
// Silent for the same reason remove is: a parent link is not a state change the
// fleet needs pushed, and re-publishing a row the visibility rule might hide is
// a separate decision from making the read correct.
func (s *sessionStore) restamp(id string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	snap, ok := s.m[id]
	if !ok || s.enrich == nil {
		return
	}
	s.m[id] = s.enrich(snap)
}
