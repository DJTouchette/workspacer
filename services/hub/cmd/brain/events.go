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
