package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"sync"
	"time"

	"github.com/coder/websocket"
)

// Frame mirrors the hub's bus.Frame wire format (internal/bus/bus.go). Only the
// fields we send/receive are kept; JSON tags match the hub exactly.
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
// so we only set type/source/data when emitting.
type Envelope struct {
	ID     string          `json:"id,omitempty"`
	Type   string          `json:"type"`
	Source string          `json:"source,omitempty"`
	Time   time.Time       `json:"time,omitempty"`
	Data   json.RawMessage `json:"data,omitempty"`
}

const source = "rules-engine"

// Topics the engine listens to. command.* is included so rules can react to
// commands others publish (and to rules' own emit/command actions).
var subscribeTopics = []string{"agent.*", "ui.*", "command.*"}

// busClient is a reconnecting WebSocket client to the hub bus. It serializes
// writes (coder/websocket forbids concurrent writers) and correlates calls by id.
type busClient struct {
	url string
	eng *engine

	mu   sync.Mutex // guards conn + writes
	conn *websocket.Conn

	pendMu  sync.Mutex
	pending map[string]chan Frame
	seq     int
}

func newBusClient(url string, eng *engine) *busClient {
	return &busClient{url: url, eng: eng, pending: map[string]chan Frame{}}
}

// run connects and reads frames until ctx is cancelled, reconnecting with backoff.
func (b *busClient) run(ctx context.Context) {
	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		if err := b.session(ctx); err != nil && ctx.Err() == nil {
			log.Printf("rules-engine: bus disconnected (%v); reconnecting in %s", err, backoff)
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

// session runs one connection: dial, subscribe, then read until error.
func (b *busClient) session(ctx context.Context) error {
	dialCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(dialCtx, b.url, nil)
	if err != nil {
		return err
	}
	// Allow large event payloads (agents.list results can be sizable).
	conn.SetReadLimit(4 << 20)

	b.mu.Lock()
	b.conn = conn
	b.mu.Unlock()
	defer func() {
		b.mu.Lock()
		b.conn = nil
		b.mu.Unlock()
		conn.CloseNow()
		b.failPending(errors.New("disconnected"))
	}()

	if err := b.write(ctx, Frame{Op: "subscribe", Topics: subscribeTopics}); err != nil {
		return err
	}
	log.Printf("rules-engine: connected to bus, subscribed to %v", subscribeTopics)

	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return err
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil {
			continue
		}
		b.dispatch(f)
	}
}

func (b *busClient) dispatch(f Frame) {
	switch f.Op {
	case "event":
		if f.Event != nil {
			b.eng.onEvent(*f.Event)
		}
	case "result", "error":
		b.pendMu.Lock()
		ch, ok := b.pending[f.ID]
		if ok {
			delete(b.pending, f.ID)
		}
		b.pendMu.Unlock()
		if ok {
			ch <- f
		}
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
		log.Printf("rules-engine: publish %s failed: %v", eventType, err)
	}
}

// call invokes a capability and waits for the result. Returns the raw result
// JSON, or an error (including the hub's "no provider for …" when the app side
// is not connected).
func (b *busClient) call(method string, params json.RawMessage) (json.RawMessage, error) {
	b.pendMu.Lock()
	b.seq++
	id := "c" + itoa(b.seq)
	ch := make(chan Frame, 1)
	b.pending[id] = ch
	b.pendMu.Unlock()

	if err := b.write(context.Background(), Frame{Op: "call", ID: id, Method: method, Params: params}); err != nil {
		b.pendMu.Lock()
		delete(b.pending, id)
		b.pendMu.Unlock()
		return nil, err
	}

	select {
	case f := <-ch:
		if f.Op == "error" {
			return nil, errors.New(f.Error)
		}
		return f.Result, nil
	case <-time.After(8 * time.Second):
		b.pendMu.Lock()
		delete(b.pending, id)
		b.pendMu.Unlock()
		return nil, errors.New("call timeout")
	}
}

func (b *busClient) failPending(err error) {
	b.pendMu.Lock()
	for id, ch := range b.pending {
		ch <- Frame{Op: "error", ID: id, Error: err.Error()}
		delete(b.pending, id)
	}
	b.pendMu.Unlock()
}

// itoa is a tiny dependency-free int→string (avoid pulling strconv for one use).
func itoa(n int) string {
	if n == 0 {
		return "0"
	}
	var buf [20]byte
	i := len(buf)
	for n > 0 {
		i--
		buf[i] = byte('0' + n%10)
		n /= 10
	}
	return string(buf[i:])
}
