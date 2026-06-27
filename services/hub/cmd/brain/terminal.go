package main

// PTY streaming over the bus — the live terminal. For each session a client is
// watching, the brain runs one SSE consumer of claudemon's
// `GET /sessions/:id/stream` (base64 chunks, first frame replays the ring
// buffer) and republishes every chunk onto the bus as `pty.bytes.<sessionId>`
// events. Input/resize flow back through the existing sessions.terminalInput /
// terminalResize capabilities. A Go port of terminalShare.ts.
//
// Streaming is lease-gated: a client calls attachTerminal when it opens the
// terminal view, refreshes with terminalKeepalive on a timer, and the stream
// stops when the lease lapses — so the brain never streams a session nobody is
// watching.

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"sync"
	"time"
)

const (
	leaseTTL   = 20 * time.Second
	sweepEvery = 5 * time.Second
	// Coalesce PTY chunks into one bus event per frame (~60fps) so bursty output
	// doesn't flood a phone with thousands of tiny messages; flush early once
	// enough piles up to bound latency.
	flushEvery = 16 * time.Millisecond
	flushBytes = 64 * 1024
)

type terminalHub struct {
	cm      *claudemonClient
	publish func(eventType string, data json.RawMessage)

	mu  sync.Mutex
	fwd map[string]*forwarder
}

func newTerminalHub(cm *claudemonClient, publish func(string, json.RawMessage)) *terminalHub {
	return &terminalHub{cm: cm, publish: publish, fwd: map[string]*forwarder{}}
}

// attach (re)starts forwarding a session's PTY and takes a fresh lease.
// Restarting is intentional: the new SSE connection replays the ring buffer,
// re-priming whichever viewer just attached.
func (h *terminalHub) attach(id string) {
	h.detach(id)
	ctx, cancel := context.WithCancel(context.Background())
	f := &forwarder{hub: h, id: id, cancel: cancel, deadline: time.Now().Add(leaseTTL)}
	h.mu.Lock()
	h.fwd[id] = f
	h.mu.Unlock()
	go f.run(ctx)
}

// keepalive refreshes the lease; false means no forwarder is active (lapsed), so
// the caller knows to re-attach (and re-prime) rather than assume it's live.
func (h *terminalHub) keepalive(id string) bool {
	h.mu.Lock()
	f, ok := h.fwd[id]
	h.mu.Unlock()
	if !ok {
		return false
	}
	f.mu.Lock()
	f.deadline = time.Now().Add(leaseTTL)
	f.mu.Unlock()
	return true
}

func (h *terminalHub) detach(id string) {
	h.mu.Lock()
	f, ok := h.fwd[id]
	if ok {
		delete(h.fwd, id)
	}
	h.mu.Unlock()
	if ok {
		f.stop()
	}
}

// sweep stops forwarders whose lease has lapsed. Runs for the brain's lifetime.
func (h *terminalHub) sweep(ctx context.Context) {
	t := time.NewTicker(sweepEvery)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-t.C:
			h.sweepExpired(now)
		}
	}
}

// sweepExpired detaches every forwarder whose lease lapsed before `now`.
func (h *terminalHub) sweepExpired(now time.Time) {
	var expired []string
	h.mu.Lock()
	for id, f := range h.fwd {
		f.mu.Lock()
		lapsed := now.After(f.deadline)
		f.mu.Unlock()
		if lapsed {
			expired = append(expired, id)
		}
	}
	h.mu.Unlock()
	for _, id := range expired {
		h.detach(id)
	}
}

type forwarder struct {
	hub    *terminalHub
	id     string
	cancel context.CancelFunc

	mu       sync.Mutex
	deadline time.Time
	pending  []byte
	timer    *time.Timer
}

func (f *forwarder) stop() {
	f.cancel()
	f.mu.Lock()
	if f.timer != nil {
		f.timer.Stop()
		f.timer = nil
	}
	f.mu.Unlock()
}

func (f *forwarder) run(ctx context.Context) {
	backoff := 500 * time.Millisecond
	for {
		if ctx.Err() != nil {
			return
		}
		_ = f.hub.cm.streamSSE(ctx, "/sessions/"+f.id+"/stream", func(name string, data []byte) {
			if name != "pty.bytes" && name != "" {
				return
			}
			chunk, err := base64.StdEncoding.DecodeString(string(data))
			if err != nil {
				return
			}
			f.onChunk(chunk)
		})
		f.flush() // emit any tail before reconnecting/stopping
		if ctx.Err() != nil {
			return
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(backoff):
		}
		if backoff < 5*time.Second {
			backoff *= 2
		}
	}
}

// onChunk appends decoded bytes and coalesces: flush on the frame timer, or
// immediately once enough has piled up. (Base64 strings can't be concatenated
// directly — only byte concatenation is correct — so we buffer decoded bytes.)
func (f *forwarder) onChunk(chunk []byte) {
	f.mu.Lock()
	f.pending = append(f.pending, chunk...)
	if len(f.pending) >= flushBytes {
		f.mu.Unlock()
		f.flush()
		return
	}
	if f.timer == nil {
		f.timer = time.AfterFunc(flushEvery, f.flush)
	}
	f.mu.Unlock()
}

func (f *forwarder) flush() {
	f.mu.Lock()
	if f.timer != nil {
		f.timer.Stop()
		f.timer = nil
	}
	if len(f.pending) == 0 {
		f.mu.Unlock()
		return
	}
	b := f.pending
	f.pending = nil
	f.mu.Unlock()

	// The bus event's data is a JSON string carrying the base64 chunk.
	payload, err := json.Marshal(base64.StdEncoding.EncodeToString(b))
	if err != nil {
		return
	}
	f.hub.publish("pty.bytes."+f.id, payload)
}
