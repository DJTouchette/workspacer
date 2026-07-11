package main

// Provider-side bus client for the hub: a reconnecting WebSocket client that,
// on connect, `register`s a set of capability methods and then answers inbound
// `call` frames via a handler. This is the seam the brain plugs into — the hub
// routes a caller's `call` to whichever provider registered the method, and we
// reply with the result. (See services/hub/README.md "Protocol".)

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net/url"
	"sync"
	"time"

	"github.com/coder/websocket"
)

// frame mirrors the hub's wire format (internal/bus/bus.go). We keep only the
// fields we send/receive.
type frame struct {
	Op     string    `json:"op"`
	Event  *envelope `json:"event,omitempty"`
	ID     string    `json:"id,omitempty"`
	Method string    `json:"method,omitempty"`

	Methods []string        `json:"methods,omitempty"`
	Params  json.RawMessage `json:"params,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   string          `json:"error,omitempty"`
}

// envelope mirrors the hub's event.Envelope. The hub stamps id/time on publish.
type envelope struct {
	Type   string          `json:"type"`
	Source string          `json:"source,omitempty"`
	Data   json.RawMessage `json:"data,omitempty"`
}

const source = "workspacer.brain"

// callHandler answers an incoming capability call. It returns the result JSON,
// or an error the hub relays back to the caller.
type callHandler func(ctx context.Context, method string, params json.RawMessage) (json.RawMessage, error)

// busClient is a reconnecting provider connection to the hub bus. Writes are
// serialized — coder/websocket forbids concurrent writers.
type busClient struct {
	url     string
	token   string
	methods []string
	handler callHandler

	mu   sync.Mutex
	conn *websocket.Conn

	// Outbound calls we made as a *caller* (e.g. the hub-local layout.get),
	// keyed by our own frame id, each waiting on its reply frame.
	callMu sync.Mutex
	calls  map[string]chan frame
	seq    int
}

func newBusClient(rawURL, token string, methods []string, handler callHandler) *busClient {
	return &busClient{url: rawURL, token: token, methods: methods, handler: handler, calls: map[string]chan frame{}}
}

// dialURL appends the auth token as a query param when set. The hub treats the
// `?token=` form as canonical (a browser WS handshake can't set headers).
func (b *busClient) dialURL() string {
	if b.token == "" {
		return b.url
	}
	u, err := url.Parse(b.url)
	if err != nil {
		return b.url
	}
	q := u.Query()
	q.Set("token", b.token)
	u.RawQuery = q.Encode()
	return u.String()
}

// run connects and reads until ctx is cancelled, reconnecting with backoff.
func (b *busClient) run(ctx context.Context) {
	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		if err := b.session(ctx); err != nil && ctx.Err() == nil {
			log.Printf("brain: bus disconnected (%v); reconnecting in %s", err, backoff)
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

// session runs one connection: dial, register methods, then read until error.
func (b *busClient) session(ctx context.Context) error {
	dialCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(dialCtx, b.dialURL(), nil)
	if err != nil {
		return err
	}
	conn.SetReadLimit(8 << 20) // transcripts/conversations can be large

	b.mu.Lock()
	b.conn = conn
	b.mu.Unlock()
	defer func() {
		b.mu.Lock()
		b.conn = nil
		b.mu.Unlock()
		conn.CloseNow()
	}()

	if err := b.write(ctx, frame{Op: "register", Methods: b.methods}); err != nil {
		return err
	}
	log.Printf("brain: connected to bus, registered %d method(s)", len(b.methods))

	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return err
		}
		var f frame
		if err := json.Unmarshal(data, &f); err != nil {
			continue
		}
		b.dispatch(ctx, f)
	}
}

func (b *busClient) dispatch(ctx context.Context, f frame) {
	switch f.Op {
	case "call":
		// Handle off the read loop: a spawn/message round-trips to claudemon and
		// could take a moment; blocking here would stall every other inbound call.
		go b.handleCall(ctx, f)
	case "result", "error":
		// The reply to one of our own outbound calls (we never *receive* replies
		// to frames we answered as a provider — the hub routes those to the
		// caller). Route it to the waiting call by id; unknown ids are dropped.
		b.callMu.Lock()
		ch := b.calls[f.ID]
		b.callMu.Unlock()
		if ch != nil {
			select {
			case ch <- f:
			default:
			}
		}
	case "registered", "hello":
		// ack; nothing to do
	}
}

// call invokes a bus method as a CALLER — the inverse of the provider role —
// used for hub-local state like the shared layout document (layout.get).
// Errors when disconnected, on an error reply, or when ctx expires.
func (b *busClient) call(ctx context.Context, method string, params any) (json.RawMessage, error) {
	raw, err := json.Marshal(params)
	if err != nil {
		return nil, err
	}
	b.callMu.Lock()
	b.seq++
	// Our ids are prefixed so they can never collide with the hub-assigned ids
	// of the inbound calls we answer (mirrors hubClient.ts's 'm' prefix).
	id := fmt.Sprintf("brain-%d", b.seq)
	ch := make(chan frame, 1)
	b.calls[id] = ch
	b.callMu.Unlock()
	defer func() {
		b.callMu.Lock()
		delete(b.calls, id)
		b.callMu.Unlock()
	}()
	if err := b.write(ctx, frame{Op: "call", ID: id, Method: method, Params: raw}); err != nil {
		return nil, err
	}
	select {
	case f := <-ch:
		if f.Op == "error" {
			return nil, errors.New(f.Error)
		}
		return f.Result, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// handleCall runs the handler and replies with result/error, echoing the
// hub-assigned id.
func (b *busClient) handleCall(ctx context.Context, f frame) {
	result, err := b.handler(ctx, f.Method, f.Params)
	reply := frame{Op: "result", ID: f.ID, Result: result}
	if err != nil {
		reply = frame{Op: "error", ID: f.ID, Error: err.Error()}
	}
	if werr := b.write(context.Background(), reply); werr != nil {
		log.Printf("brain: reply to call %s (%s) failed: %v", f.ID, f.Method, werr)
	}
}

// publish emits a fire-and-forget event onto the bus. Best-effort: if we're not
// connected yet (e.g. during initial seeding) the write fails and is dropped.
func (b *busClient) publish(eventType string, data json.RawMessage) {
	_ = b.write(context.Background(), frame{
		Op:    "publish",
		Event: &envelope{Type: eventType, Source: source, Data: data},
	})
}

// write serializes a frame onto the connection.
func (b *busClient) write(ctx context.Context, f frame) error {
	data, err := json.Marshal(f)
	if err != nil {
		return err
	}
	b.mu.Lock()
	conn := b.conn
	b.mu.Unlock()
	if conn == nil {
		return errors.New("not connected")
	}
	wctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	return conn.Write(wctx, websocket.MessageText, data)
}
