package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"net/url"
	"sync"
	"time"

	"github.com/coder/websocket"
)

// Frame mirrors the hub's bus.Frame wire format (internal/bus/bus.go). JSON tags
// match the hub exactly; we keep only the fields we send/receive.
type Frame struct {
	Op     string    `json:"op"`
	Topics []string  `json:"topics,omitempty"`
	Event  *Envelope `json:"event,omitempty"`

	ID      string          `json:"id,omitempty"`
	Method  string          `json:"method,omitempty"`
	Methods []string        `json:"methods,omitempty"`
	Params  json.RawMessage `json:"params,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   string          `json:"error,omitempty"`
}

// Envelope mirrors the hub's event.Envelope. The hub stamps id/time on publish,
// so we set only type/source/data when emitting.
type Envelope struct {
	ID     string          `json:"id,omitempty"`
	Type   string          `json:"type"`
	Source string          `json:"source,omitempty"`
	Time   time.Time       `json:"time,omitempty"`
	Data   json.RawMessage `json:"data,omitempty"`
}

const source = "workspacer.rivet-bridge"

// callHandler answers an incoming bus capability call. It returns the result
// JSON, or an error that the hub relays to the caller.
type callHandler func(ctx context.Context, method string, params json.RawMessage) (json.RawMessage, error)

// busClient is a reconnecting WebSocket client to the hub bus. Unlike the
// rules-engine's (a caller), this one is a capability PROVIDER: on connect it
// registers its methods, then answers incoming "call" frames via handler.
// Writes are serialized (coder/websocket forbids concurrent writers).
type busClient struct {
	url     string
	token   string // optional bus auth token (HUB_TOKEN); sent as ?token=
	methods []string
	handler callHandler

	mu   sync.Mutex // guards conn + writes
	conn *websocket.Conn
}

func newBusClient(rawURL, token string, methods []string, handler callHandler) *busClient {
	return &busClient{url: rawURL, token: token, methods: methods, handler: handler}
}

// dialURL appends the auth token as a query param when set. The hub accepts
// either `?token=` or `Authorization: Bearer`; browsers can't set headers on a
// WS handshake, so the hub treats the query form as canonical and we match it.
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

// run connects and reads frames until ctx is cancelled, reconnecting with backoff.
func (b *busClient) run(ctx context.Context) {
	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		if err := b.session(ctx); err != nil && ctx.Err() == nil {
			log.Printf("rivet-bridge: bus disconnected (%v); reconnecting in %s", err, backoff)
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
	conn.SetReadLimit(8 << 20) // recon/schema outputs can be large

	b.mu.Lock()
	b.conn = conn
	b.mu.Unlock()
	defer func() {
		b.mu.Lock()
		b.conn = nil
		b.mu.Unlock()
		conn.CloseNow()
	}()

	if err := b.write(ctx, Frame{Op: "register", Methods: b.methods}); err != nil {
		return err
	}
	log.Printf("rivet-bridge: connected to bus, registered %d method(s)", len(b.methods))

	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return err
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil {
			continue
		}
		b.dispatch(ctx, f)
	}
}

func (b *busClient) dispatch(ctx context.Context, f Frame) {
	switch f.Op {
	case "call":
		// Handle off the read loop: a tools/call can take seconds, and blocking
		// here would stall every other inbound frame (and the next call).
		go b.handleCall(ctx, f)
	case "registered":
		// ack; nothing to do
	}
}

// handleCall runs the provider handler and replies with result/error, echoing
// the hub-assigned id.
func (b *busClient) handleCall(ctx context.Context, f Frame) {
	result, err := b.handler(ctx, f.Method, f.Params)
	reply := Frame{Op: "result", ID: f.ID, Result: result}
	if err != nil {
		reply = Frame{Op: "error", ID: f.ID, Error: err.Error()}
	}
	if werr := b.write(context.Background(), reply); werr != nil {
		log.Printf("rivet-bridge: reply to call %s (%s) failed: %v", f.ID, f.Method, werr)
	}
}

// write serializes a frame onto the connection.
func (b *busClient) write(ctx context.Context, f Frame) error {
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

// publish emits an event (fire-and-forget).
func (b *busClient) publish(eventType string, data json.RawMessage) {
	if err := b.write(context.Background(), Frame{
		Op:    "publish",
		Event: &Envelope{Type: eventType, Source: source, Data: data},
	}); err != nil {
		log.Printf("rivet-bridge: publish %s failed: %v", eventType, err)
	}
}
