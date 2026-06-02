// Package bus exposes the broker over a WebSocket endpoint. Each client holds a
// single bidirectional connection used for two things:
//
//   - events  — subscribe to topics; publish events (pub/sub, via the broker)
//   - calls   — invoke capabilities other clients provide (request/reply, via
//               the router): a provider registers method names, a caller calls
//               them, the hub routes the call and its result between them.
//
// The hub never implements capabilities; it routes them. That keeps the control
// plane generic and is exactly the seam the MCP facade plugs into.
package bus

import (
	"context"
	"encoding/json"
	"net/http"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

const writeTimeout = 5 * time.Second

// Frame is the wire message exchanged with a client.
//
//	client -> hub:  subscribe | unsubscribe | publish | register | call | result | error
//	hub -> client:  hello | subscribed | unsubscribed | event | registered |
//	                call | result | error
type Frame struct {
	Op     string          `json:"op"`
	Topics []string        `json:"topics,omitempty"`
	Event  *event.Envelope `json:"event,omitempty"`

	// RPC fields.
	ID      string          `json:"id,omitempty"`      // correlation id
	Method  string          `json:"method,omitempty"`  // call
	Methods []string        `json:"methods,omitempty"` // register
	Params  json.RawMessage `json:"params,omitempty"`  // call
	Result  json.RawMessage `json:"result,omitempty"`  // result
	Error   string          `json:"error,omitempty"`   // error
}

// Server adapts a broker (events) + router (calls) to HTTP/WebSocket.
type Server struct {
	broker *broker.Broker
	router *router
	extra  map[string]http.HandlerFunc
}

// NewServer wraps a broker.
func NewServer(b *broker.Broker) *Server {
	return &Server{broker: b, router: newRouter(), extra: map[string]http.HandlerFunc{}}
}

// AddRoute registers an extra HTTP route (e.g. /plugins). Call before Handler().
// Keeps the bus package decoupled from what it serves alongside the bus.
func (s *Server) AddRoute(path string, h http.HandlerFunc) {
	s.extra[path] = h
}

// Handler returns the routed HTTP handler (/bus WebSocket, /health JSON, extras).
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/bus", s.handleBus)
	mux.HandleFunc("/health", s.handleHealth)
	for path, h := range s.extra {
		mux.HandleFunc(path, h)
	}
	return mux
}

func (s *Server) handleHealth(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"status":      "ok",
		"subscribers": s.broker.SubscriberCount(),
		"methods":     s.router.methodCount(),
	})
}

func (s *Server) handleBus(w http.ResponseWriter, r *http.Request) {
	ws, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
	if err != nil {
		return
	}
	ctx := r.Context()
	cn := &conn{ws: ws, ctx: ctx}
	s.router.addConn(cn)
	defer cn.ws.CloseNow()

	sub := s.broker.Subscribe(nil)
	defer s.broker.Unsubscribe(sub)
	defer s.router.dropConn(cn) // unregister provider + fail outstanding calls

	// Writer goroutine: pump matched events to this client. Blocking here (a
	// slow TCP client) only backs up this subscriber's buffer — the broker
	// drops past capacity, so other clients and publishers are unaffected.
	go func() {
		for ev := range sub.C {
			ev := ev
			if err := cn.send(Frame{Op: "event", Event: &ev}); err != nil {
				return
			}
		}
	}()

	_ = cn.send(Frame{Op: "hello"})

	for {
		_, data, err := ws.Read(ctx)
		if err != nil {
			return
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil {
			_ = cn.send(Frame{Op: "error", Error: "bad frame: " + err.Error()})
			continue
		}
		switch f.Op {
		case "subscribe":
			sub.AddTopics(f.Topics...)
			_ = cn.send(Frame{Op: "subscribed", Topics: sub.Topics()})
		case "unsubscribe":
			sub.RemoveTopics(f.Topics...)
			_ = cn.send(Frame{Op: "unsubscribed", Topics: sub.Topics()})
		case "publish":
			if f.Event == nil {
				_ = cn.send(Frame{Op: "error", Error: "publish missing event"})
				continue
			}
			s.broker.Publish(*f.Event)
		case "register":
			s.router.register(cn, f.Methods)
			_ = cn.send(Frame{Op: "registered", Methods: f.Methods})
		case "call":
			s.router.call(cn, f)
		case "result":
			s.router.result(cn, f, false)
		case "error":
			s.router.result(cn, f, true)
		default:
			_ = cn.send(Frame{Op: "error", Error: "unknown op: " + f.Op})
		}
	}
}

// conn serializes writes; coder/websocket forbids concurrent writers, and the
// writer goroutine, read loop, and router all emit frames.
type conn struct {
	id      uint64
	ws      *websocket.Conn
	ctx     context.Context
	writeMu sync.Mutex
}

func (cn *conn) send(f Frame) error {
	data, err := json.Marshal(f)
	if err != nil {
		return err
	}
	cn.writeMu.Lock()
	defer cn.writeMu.Unlock()
	wctx, cancel := context.WithTimeout(cn.ctx, writeTimeout)
	defer cancel()
	return cn.ws.Write(wctx, websocket.MessageText, data)
}
