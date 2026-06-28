// Package bus exposes the broker over a WebSocket endpoint. Each client holds a
// single bidirectional connection used for two things:
//
//   - events  — subscribe to topics; publish events (pub/sub, via the broker)
//   - calls   — invoke capabilities other clients provide (request/reply, via
//     the router): a provider registers method names, a caller calls
//     them, the hub routes the call and its result between them.
//
// The hub never implements capabilities; it routes them. That keeps the control
// plane generic and is exactly the seam the MCP facade plugs into.
package bus

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
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

// pluginIdent is the identity a per-plugin bus token resolves to: which plugin,
// and the grants it holds — the capabilities it may call, each with optional
// filesystem scoping.
type pluginIdent struct {
	id   string
	caps map[string]capGrant
}

// capGrant is what a plugin token may do with one capability. fsRoots, when
// non-empty, confines a path-scoped call (fs.*, search.project) to targets
// inside one of these canonical roots; empty means the method carries no path to
// confine (driving, observation, notifications, …).
type capGrant struct {
	fsRoots []string
}

// Server adapts a broker (events) + router (calls) to HTTP/WebSocket.
type Server struct {
	broker *broker.Broker
	router *router
	extra  map[string]http.HandlerFunc
	token  string // when non-empty, /bus + protected routes require this bearer token

	// Per-plugin tokens: a connection presenting one is tagged as that plugin and
	// may only call the capabilities it declared. The host token (s.token) is
	// trusted (full access). Registered by the plugin manager.
	ptMu         sync.RWMutex
	pluginTokens map[string]pluginIdent
}

// NewServer wraps a broker.
func NewServer(b *broker.Broker) *Server {
	return &Server{broker: b, router: newRouter(), extra: map[string]http.HandlerFunc{}, pluginTokens: map[string]pluginIdent{}}
}

// RegisterPluginToken maps a per-plugin bus token to the plugin's id and the
// grants it holds. Filesystem roots are canonicalized once here (symlinks + ..
// resolved) so the per-call containment check doesn't re-walk them; a root that
// can't be canonicalized is dropped, since it can't safely grant anything.
// Idempotent; called by the plugin manager on load.
func (s *Server) RegisterPluginToken(token, pluginID string, grants []capspec.Grant) {
	if token == "" {
		return
	}
	set := make(map[string]capGrant, len(grants))
	for _, g := range grants {
		if g.Method == "" {
			continue
		}
		set[g.Method] = capGrant{fsRoots: canonRoots(g.FSRoots)}
	}
	s.ptMu.Lock()
	s.pluginTokens[token] = pluginIdent{id: pluginID, caps: set}
	s.ptMu.Unlock()
}

// canonRoots canonicalizes grant roots once at registration, dropping any that
// don't resolve (a root that isn't a real path can't confine anything safely).
func canonRoots(roots []string) []string {
	if len(roots) == 0 {
		return nil
	}
	out := make([]string, 0, len(roots))
	for _, r := range roots {
		if c, err := canonicalize(r); err == nil {
			out = append(out, c)
		}
	}
	return out
}

// UnregisterPluginToken drops a plugin token (on unload/replace).
func (s *Server) UnregisterPluginToken(token string) {
	if token == "" {
		return
	}
	s.ptMu.Lock()
	delete(s.pluginTokens, token)
	s.ptMu.Unlock()
}

func (s *Server) lookupPluginToken(token string) (pluginIdent, bool) {
	s.ptMu.RLock()
	defer s.ptMu.RUnlock()
	pi, ok := s.pluginTokens[token]
	return pi, ok
}

// SetToken sets the shared secret required to connect to /bus (and to call
// Authorized). Empty token = no auth (the localhost-only default). Set this
// whenever the bus is reachable beyond loopback (remote sharing / Tailscale).
func (s *Server) SetToken(t string) {
	s.token = t
}

// Authorized reports whether a request carries the required token. Always true
// when no token is configured. Accepts either `Authorization: Bearer <token>`
// or a `?token=<token>` query param — browsers can't set headers on a
// WebSocket handshake, so the query form is what the mobile client uses.
func (s *Server) Authorized(r *http.Request) bool {
	if s.token == "" {
		return true
	}
	if h := r.Header.Get("Authorization"); h == "Bearer "+s.token {
		return true
	}
	return r.URL.Query().Get("token") == s.token
}

// presentedToken extracts the caller's token from an Authorization: Bearer
// header or a ?token= query param (WebSocket handshakes can't set headers from
// a browser, so webview clients use the query form).
func presentedToken(r *http.Request) string {
	if h := r.Header.Get("Authorization"); strings.HasPrefix(h, "Bearer ") {
		return strings.TrimPrefix(h, "Bearer ")
	}
	return r.URL.Query().Get("token")
}

// RegisterLocal installs an in-process capability handler so the hub itself can
// provide a method (e.g. the shared layout document) without a WebSocket
// provider. Local handlers take precedence over remote providers of the same
// name. Call before Handler().
func (s *Server) RegisterLocal(method string, h LocalHandler) {
	s.router.registerLocal(method, h)
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
	// Classify the connection by the token it presents:
	//   - a registered per-plugin token → that plugin, restricted to its caps
	//   - the host token (or no host token configured) → trusted, full access
	//   - anything else → rejected
	tok := presentedToken(r)
	var trusted bool
	var caps map[string]capGrant
	var pluginID string
	if pi, ok := s.lookupPluginToken(tok); ok {
		caps, pluginID = pi.caps, pi.id
	} else if s.token == "" || tok == s.token {
		trusted = true
	} else {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	ws, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
	if err != nil {
		return
	}
	// coder/websocket defaults to a 32 KiB per-message read limit, which silently
	// kills the connection on any larger frame. RPC results carry full payloads —
	// a session transcript is easily hundreds of KB — so lift the cap well clear
	// of any realistic frame. Applies to the trusted main-process provider link
	// and to token-gated remote clients alike.
	ws.SetReadLimit(64 << 20) // 64 MiB
	ctx := r.Context()
	cn := &conn{ws: ws, ctx: ctx, trusted: trusted, caps: caps, pluginID: pluginID}
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

	// Capability authorization, set at handshake. A trusted conn (host token) may
	// call anything; a plugin conn may call only the methods it was granted, and
	// path-scoped ones only within their granted roots.
	trusted  bool
	caps     map[string]capGrant
	pluginID string
}

// mayCall reports whether this connection is allowed to invoke method at all
// (the verb check). Trusted connections (the host / MCP facade) may call
// anything; a plugin may call only the capabilities it was granted. Argument
// scoping (which paths) is a separate step — see authorize.
func (cn *conn) mayCall(method string) bool {
	if cn.trusted {
		return true
	}
	_, ok := cn.caps[method]
	return ok
}

// authorize enforces argument-level scoping for a call mayCall already admitted.
// Trusted conns are unrestricted. For a path-scoped method, the call's path is
// canonicalized and must fall inside the grant's roots; anything that can't be
// verified (missing field, no roots, resolution error) is denied — fail closed.
// Non-path methods pass straight through.
func (cn *conn) authorize(method string, params json.RawMessage) error {
	if cn.trusted {
		return nil
	}
	g, ok := cn.caps[method]
	if !ok {
		return fmt.Errorf("plugin not authorized for capability %s", method)
	}
	field, scoped := capspec.IsPathScoped(method)
	if !scoped {
		return nil // verb-only capability; mayCall already governs it
	}
	if len(g.fsRoots) == 0 {
		return fmt.Errorf("%s: filesystem-scoped capability granted with no roots", method)
	}
	target, ok := paramString(params, field)
	if !ok {
		return fmt.Errorf("%s: missing %q for filesystem-scoped capability", method, field)
	}
	within, err := pathWithinRoots(g.fsRoots, target)
	if err != nil {
		return fmt.Errorf("%s: cannot resolve %q: %w", method, target, err)
	}
	if !within {
		return fmt.Errorf("%s: path %q is outside the plugin's granted scope", method, target)
	}
	return nil
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
