// Package busclient is a small WebSocket client for the workspacer hub bus.
//
// It connects as a capability *caller*: it invokes methods that other bus
// clients (the Electron main process) provide, correlating each call with its
// reply by id. The hub routes the call to the owning provider and the result
// back here. This is the seam the MCP facade uses to reach workspacer — it
// never touches workspacer state directly, it just calls capabilities.
//
// The client maintains a single connection, reconnecting with backoff, and is
// safe for concurrent use by many callers.
package busclient

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

var (
	// ErrNotConnected is returned when a call is made while the link to the hub
	// is down and does not come back within the readiness window.
	ErrNotConnected = errors.New("busclient: not connected to hub")
	// ErrConnLost is delivered to outstanding calls when the connection drops.
	ErrConnLost = errors.New("busclient: connection lost")
)

const (
	readyWait    = 5 * time.Second
	writeTimeout = 5 * time.Second
	dialTimeout  = 10 * time.Second
)

// frame mirrors the subset of the hub's wire protocol a caller needs. The hub's
// own bus.Frame is the authoritative shape; this is a deliberately minimal copy
// so the client doesn't depend on the bus package.
type frame struct {
	Op     string          `json:"op"`
	ID     string          `json:"id,omitempty"`
	Method string          `json:"method,omitempty"`
	Topics []string        `json:"topics,omitempty"`
	Params json.RawMessage `json:"params,omitempty"`
	Result json.RawMessage `json:"result,omitempty"`
	Error  string          `json:"error,omitempty"`
	Event  *event.Envelope `json:"event,omitempty"`
}

type reply struct {
	result json.RawMessage
	err    error
}

// Client maintains one connection to the hub bus and routes call replies back
// to their callers.
type Client struct {
	url string

	mu      sync.Mutex
	conn    *websocket.Conn
	seq     uint64
	pending map[string]chan reply
	ready   bool
	// Subscriptions survive the connection: re-sent on every (re)connect, the
	// same contract hubClient.ts keeps on the desktop side.
	subs    []string
	onEvent func(event.Envelope)

	writeMu sync.Mutex // coder/websocket forbids concurrent writers
}

// New builds a client for the given bus URL (e.g. ws://127.0.0.1:7895/bus).
// When token is non-empty it is appended as a ?token= query param — the form
// the hub accepts on the WebSocket handshake (headers can't ride a browser WS
// upgrade, and the hub treats both equivalently).
func New(busURL, token string) *Client {
	if token != "" {
		sep := "?"
		if strings.Contains(busURL, "?") {
			sep = "&"
		}
		busURL += sep + "token=" + url.QueryEscape(token)
	}
	return &Client{url: busURL, pending: make(map[string]chan reply)}
}

// Ready reports whether the client currently holds a live connection.
func (c *Client) Ready() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.ready && c.conn != nil
}

// OnEvent sets the handler for inbound events. Set it BEFORE Run so no event
// races the registration; the handler is invoked from the read loop, so it must
// not block (hand off to a channel/goroutine if the work is slow — a stalled
// handler stalls call replies too, they share the loop).
func (c *Client) OnEvent(fn func(event.Envelope)) {
	c.mu.Lock()
	c.onEvent = fn
	c.mu.Unlock()
}

// Subscribe adds topic patterns (exact, `prefix.*`, or `*`) to the client's
// subscription set and, when connected, sends them immediately. The full set is
// re-sent on every reconnect, so a Subscribe survives connection drops — the
// property federation links depend on.
func (c *Client) Subscribe(patterns ...string) {
	if len(patterns) == 0 {
		return
	}
	c.mu.Lock()
	have := make(map[string]bool, len(c.subs))
	for _, s := range c.subs {
		have[s] = true
	}
	var fresh []string
	for _, p := range patterns {
		if p != "" && !have[p] {
			have[p] = true
			c.subs = append(c.subs, p)
			fresh = append(fresh, p)
		}
	}
	conn := c.conn
	connected := c.ready && conn != nil
	c.mu.Unlock()
	if !connected || len(fresh) == 0 {
		return
	}
	out, _ := json.Marshal(frame{Op: "subscribe", Topics: fresh})
	c.writeMu.Lock()
	wctx, cancel := context.WithTimeout(context.Background(), writeTimeout)
	_ = conn.Write(wctx, websocket.MessageText, out)
	cancel()
	c.writeMu.Unlock()
}

// Run maintains the connection until ctx is cancelled, reconnecting with
// backoff. It blocks, so callers typically run it in a goroutine.
func (c *Client) Run(ctx context.Context) {
	backoff := 200 * time.Millisecond
	for ctx.Err() == nil {
		start := time.Now()
		c.connectAndRead(ctx)
		if ctx.Err() != nil {
			return
		}
		// A connection that lived a while is a fresh failure, not a tight loop —
		// reset the backoff so a long-lived link reconnects promptly.
		if time.Since(start) > 5*time.Second {
			backoff = 200 * time.Millisecond
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

// connectAndRead dials, then pumps inbound frames until the connection drops.
func (c *Client) connectAndRead(ctx context.Context) {
	dialCtx, cancel := context.WithTimeout(ctx, dialTimeout)
	conn, _, err := websocket.Dial(dialCtx, c.url, nil)
	cancel()
	if err != nil {
		return
	}
	// RPC results carry full payloads (a transcript is easily hundreds of KB),
	// so lift coder/websocket's 32 KiB default read cap well clear, matching the
	// hub server side.
	conn.SetReadLimit(64 << 20)

	c.mu.Lock()
	c.conn = conn
	subs := append([]string(nil), c.subs...)
	c.mu.Unlock()

	defer func() {
		c.mu.Lock()
		c.ready = false
		c.conn = nil
		// Fail every outstanding call so callers don't hang on a dead link.
		for id, ch := range c.pending {
			ch <- reply{err: ErrConnLost}
			delete(c.pending, id)
		}
		c.mu.Unlock()
		conn.CloseNow()
	}()

	// Re-establish subscriptions before anything else on the fresh connection,
	// so a reconnect never silently loses the event feed. ready is set only
	// once this write lands (or is skipped because there's nothing to
	// resubscribe): Ready() promises a live event feed, not just an open
	// socket, so a caller that waits on it and then publishes never races the
	// resubscribe frame still sitting unsent.
	if len(subs) > 0 {
		out, _ := json.Marshal(frame{Op: "subscribe", Topics: subs})
		c.writeMu.Lock()
		wctx, cancel := context.WithTimeout(ctx, writeTimeout)
		err := conn.Write(wctx, websocket.MessageText, out)
		cancel()
		c.writeMu.Unlock()
		if err != nil {
			return
		}
	}

	c.mu.Lock()
	c.ready = true
	c.mu.Unlock()

	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return
		}
		var f frame
		if err := json.Unmarshal(data, &f); err != nil {
			continue
		}
		switch f.Op {
		case "result", "error":
			c.deliver(f)
		case "event":
			if f.Event != nil {
				c.mu.Lock()
				fn := c.onEvent
				c.mu.Unlock()
				if fn != nil {
					fn(*f.Event)
				}
			}
			// hello / subscribed / registered: acks, nothing to route.
		}
	}
}

// deliver routes a reply frame to the waiting caller. The pending channels are
// buffered (cap 1) so this never blocks even if the caller already gave up.
func (c *Client) deliver(f frame) {
	c.mu.Lock()
	ch, ok := c.pending[f.ID]
	if ok {
		delete(c.pending, f.ID)
	}
	c.mu.Unlock()
	if !ok {
		return
	}
	if f.Op == "error" {
		ch <- reply{err: errors.New(f.Error)}
		return
	}
	ch <- reply{result: f.Result}
}

// Call invokes a hub capability and returns its raw JSON result. If the link is
// momentarily down it waits up to readyWait (bounded by ctx) for a reconnect
// before giving up with ErrNotConnected.
func (c *Client) Call(ctx context.Context, method string, params any) (json.RawMessage, error) {
	raw, err := json.Marshal(params)
	if err != nil {
		return nil, fmt.Errorf("marshal params: %w", err)
	}

	conn, id, ch, err := c.begin(ctx, method, raw)
	if err != nil {
		return nil, err
	}

	out, _ := json.Marshal(frame{Op: "call", ID: id, Method: method, Params: raw})
	c.writeMu.Lock()
	wctx, cancel := context.WithTimeout(ctx, writeTimeout)
	err = conn.Write(wctx, websocket.MessageText, out)
	cancel()
	c.writeMu.Unlock()
	if err != nil {
		c.forget(id)
		return nil, fmt.Errorf("write call: %w", err)
	}

	select {
	case <-ctx.Done():
		c.forget(id)
		return nil, ctx.Err()
	case r := <-ch:
		return r.result, r.err
	}
}

// Publish puts one event on the bus, fire-and-forget: no ack frame exists for
// a publish, so a nil return means "handed to the socket", not "delivered".
// The broker stamps the envelope's id/time. Waits briefly for a reconnect like
// Call does, so a publish issued during a blip isn't dropped silently.
func (c *Client) Publish(ctx context.Context, ev event.Envelope) error {
	conn, err := c.live(ctx)
	if err != nil {
		return err
	}
	out, _ := json.Marshal(frame{Op: "publish", Event: &ev})
	c.writeMu.Lock()
	wctx, cancel := context.WithTimeout(ctx, writeTimeout)
	err = conn.Write(wctx, websocket.MessageText, out)
	cancel()
	c.writeMu.Unlock()
	if err != nil {
		return fmt.Errorf("write publish: %w", err)
	}
	return nil
}

// live waits (bounded by readyWait/ctx) for a connected socket.
func (c *Client) live(ctx context.Context) (*websocket.Conn, error) {
	deadline := time.Now().Add(readyWait)
	for {
		c.mu.Lock()
		if c.ready && c.conn != nil {
			conn := c.conn
			c.mu.Unlock()
			return conn, nil
		}
		c.mu.Unlock()
		if time.Now().After(deadline) {
			return nil, ErrNotConnected
		}
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-time.After(50 * time.Millisecond):
		}
	}
}

// begin waits for a live connection, then registers a pending call and returns
// the connection to write on plus its reply channel.
func (c *Client) begin(ctx context.Context, _ string, _ json.RawMessage) (*websocket.Conn, string, chan reply, error) {
	deadline := time.Now().Add(readyWait)
	for {
		c.mu.Lock()
		if c.ready && c.conn != nil {
			c.seq++
			id := strconv.FormatUint(c.seq, 10)
			ch := make(chan reply, 1)
			c.pending[id] = ch
			conn := c.conn
			c.mu.Unlock()
			return conn, id, ch, nil
		}
		c.mu.Unlock()

		if time.Now().After(deadline) {
			return nil, "", nil, ErrNotConnected
		}
		select {
		case <-ctx.Done():
			return nil, "", nil, ctx.Err()
		case <-time.After(50 * time.Millisecond):
		}
	}
}

func (c *Client) forget(id string) {
	c.mu.Lock()
	delete(c.pending, id)
	c.mu.Unlock()
}
