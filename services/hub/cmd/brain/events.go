package main

// Live session-store runner: seed from claudemon's /sessions, then follow its
// /events SSE stream, refreshing each changed session's canonical snapshot and
// pushing it into the store (which publishes an `agent.snapshot` bus event).
// Reconnects with backoff. Ports the SSE consumption pattern from the hub's
// internal/claudemon bridge.

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"strings"
	"time"
)

// backoffAfterConn returns the reconnect delay to wait after a stream
// connection that lasted `lived`. A connection that stayed up a while is a
// fresh failure, not a tight loop, so it resets to the base delay; otherwise
// the caller's escalating backoff is preserved. Mirrors busclient.Run.
func backoffAfterConn(backoff, lived time.Duration) time.Duration {
	if lived > 5*time.Second {
		return time.Second
	}
	return backoff
}

// runSessionStore blocks until ctx is cancelled, keeping `store` current.
func runSessionStore(ctx context.Context, cm *claudemonClient, store *sessionStore) {
	seedStore(ctx, cm, store)

	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		start := time.Now()
		_ = cm.streamEvents(ctx, func(name string, data []byte) {
			// claudemon names its frames "session.update" (some emit no name).
			if name != "session.update" && name != "" {
				return
			}
			var su struct {
				SessionID string `json:"session_id"`
			}
			if json.Unmarshal(data, &su) != nil || su.SessionID == "" {
				return
			}
			// Refresh the canonical snapshot (same shape as /sessions, incl. the
			// usage overlay) rather than trusting the event's embedded state.
			if snap, err := cm.getSession(ctx, su.SessionID); err == nil {
				store.set(su.SessionID, snap)
			}
		})
		if ctx.Err() != nil {
			return
		}
		backoff = backoffAfterConn(backoff, time.Since(start))
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

// fleetVisibility is the shared fleet rule as this publisher consumes it.
// *visibility satisfies it; the interface exists so a test can hand the
// publisher a rule that says YES to everything and watch what the publisher
// refuses on its own.
type fleetVisibility interface {
	visible(ctx context.Context, snap json.RawMessage) bool
}

// visibleStatusLinePublisher builds runStatusLines' publish callback with the
// SAME fleet-visibility rule every other fleet read already goes through.
//
// It did not have one, and that was a disclosure the capability plane refuses.
// agents.list and sessions.snapshots run through registry.visibleSnapshots, and
// every agent.snapshot publish through vis.visible — because a stopped session
// the shared layout does not curate is deliberately not part of the fleet. This
// publish ran unconditionally: with a layout curating nothing, sessions.snapshots
// returned zero snapshots and vis.visible(HIDDEN-1) was false while
// agent.statusline published {"sessionId":"HIDDEN-1","statusLine":{model_display,
// cost_usd:41.72, context_used_pct:88.1, five_hour_pct:93.0}} to every
// subscriber. That leaks the EXISTENCE and id of a hidden session plus its cost,
// model and rate-limit state — and sessions.snapshot(id) is view-callable and
// unfiltered by id, so the leaked id completed the read.
//
// TWO INDEPENDENT QUESTIONS, and the store one is not delegable.
//
// "Does the store admit to having this session?" and "does the fleet rule show
// it?" are different questions with different failure modes, and the second
// cannot answer the first. A session the store has never seen has no snapshot to
// evaluate, so what reached the fleet rule was `nil` — and it came back false
// only because snapshotVisible's json.Unmarshal happens to error on empty
// input. That is an accident in another file, one byte away from not holding:
// the literal `null` decodes CLEANLY into the zero struct, whose mode is "" and
// whose status is not "ended", which snapshotVisible reads as LIVE and therefore
// visible. Relying on it made the arm survive deletion while the leak it stops —
// announcing the id, model, cost and rate-limit state of a session, with
// sessions.snapshot(id) view-callable and unfiltered by id — stayed one refactor
// away. So it is checked here, first, on its own terms; updateStatusLine already
// skips a session it has no snapshot for ("nothing to merge into yet"), and this
// keeps the bus agreeing with it.
//
// vis is taken as an interface so the composition above is testable: a rule that
// admits everything it is shown must still not get an unknown session published.
// A nil rule fails CLOSED — a publisher with no visibility filter is the exact
// state this function was written to end, and publishing everything would be a
// worse answer than publishing nothing.
//
// The topic is also guarded on the event plane now (it carries
// sessions.snapshot's output), but a guard on WHO receives it is not a
// substitute for a filter on WHAT is published: the two protect against
// different mistakes, and the desktop's twin publisher makes the same choice.
func visibleStatusLinePublisher(store *sessionStore, vis fleetVisibility, publish func(string, json.RawMessage)) func(string, json.RawMessage) {
	return func(id string, sl json.RawMessage) {
		if vis == nil {
			return
		}
		snap, ok := store.get(id)
		if !ok {
			return
		}
		if !vis.visible(context.Background(), snap) {
			return
		}
		payload, err := json.Marshal(map[string]any{"sessionId": id, "statusLine": sl})
		if err != nil {
			return
		}
		publish("agent.statusline", payload)
	}
}

// runStatusLines follows claudemon's high-frequency /statusline/stream, merging
// each tick into the store (silently) and handing it to publish for the light
// `agent.statusline` event. Reconnects with backoff.
func runStatusLines(ctx context.Context, cm *claudemonClient, store *sessionStore, publish func(id string, statusLine json.RawMessage)) {
	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		start := time.Now()
		_ = cm.streamStatusLines(ctx, func(name string, data []byte) {
			if name != "statusline" && name != "" {
				return
			}
			var u struct {
				SessionID  string          `json:"session_id"`
				StatusLine json.RawMessage `json:"status_line"`
			}
			if json.Unmarshal(data, &u) != nil || u.SessionID == "" {
				return
			}
			store.updateStatusLine(u.SessionID, u.StatusLine)
			if publish != nil {
				publish(u.SessionID, u.StatusLine)
			}
		})
		if ctx.Err() != nil {
			return
		}
		backoff = backoffAfterConn(backoff, time.Since(start))
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

func seedStore(ctx context.Context, cm *claudemonClient, store *sessionStore) {
	raw, err := cm.listSessions(ctx)
	if err != nil {
		return
	}
	var arr []json.RawMessage
	if json.Unmarshal(raw, &arr) != nil {
		return
	}
	seed := make(map[string]json.RawMessage, len(arr))
	for _, snap := range arr {
		if id := snapshotID(snap); id != "" {
			seed[id] = snap
		}
	}
	store.seed(seed)
}

// parseSSE reads a Server-Sent Events stream, calling emit(name, data) per
// complete event. Mirrors internal/claudemon's parser.
func parseSSE(ctx context.Context, r io.Reader, emit func(name string, data []byte)) error {
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, 64*1024), 1<<20)
	var name string
	var data []byte
	flush := func() {
		if len(data) > 0 {
			emit(name, data)
		}
		name, data = "", nil
	}
	for sc.Scan() {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		line := sc.Text()
		switch {
		case line == "":
			flush()
		case strings.HasPrefix(line, ":"):
			// comment / heartbeat
		case strings.HasPrefix(line, "event:"):
			name = strings.TrimSpace(line[len("event:"):])
		case strings.HasPrefix(line, "data:"):
			chunk := strings.TrimPrefix(line[len("data:"):], " ")
			if data != nil {
				data = append(data, '\n')
			}
			data = append(data, chunk...)
		}
	}
	flush()
	return sc.Err()
}
