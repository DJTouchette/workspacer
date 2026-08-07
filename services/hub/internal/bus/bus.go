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
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

const writeTimeout = 5 * time.Second

// maxFrameTopics caps how many patterns one subscribe/unsubscribe frame may
// carry. Both ops rewrite the subscription's topic list under the lock the
// broker's fan-out needs, so the work a single frame can demand is work every
// publisher on the bus waits for. `subscribe` is also the one op with no
// authorization check at all — deliberately open to every tier — so without a
// cap the lowest-privilege credential the system mints (a read-only `view`
// token, or a plugin granted nothing) could wedge the entire control plane with
// one ~1 MB frame. Oversized frames are rejected rather than truncated: a client
// silently keeping a fraction of what it asked for would miss events with no way
// to tell why.
const maxFrameTopics = 256

// forwardQueueDepth bounds how many cross-connection forwards one connection may
// have waiting on peers that aren't draining (see [conn.forward]). Against a
// healthy peer the queue is effectively always empty — the dispatcher drains it
// as fast as a socket write completes — so reaching this depth means the target
// has stopped reading, and a bounded backlog with a visible error beats growing
// one client's queue without limit.
const forwardQueueDepth = 256

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
	Methods []string        `json:"methods,omitempty"` // register | hello
	Params  json.RawMessage `json:"params,omitempty"`  // call
	Result  json.RawMessage `json:"result,omitempty"`  // result
	Error   string          `json:"error,omitempty"`   // error

	// Scope, on `hello`, names the tier this connection authenticated as
	// ("view" / "triage" / "operator"), with Methods carrying the patterns it may
	// call. Without it a client can only discover its own ceiling by calling
	// something and reading the deny error — so /m would have to offer buttons
	// (spawn, model switch) that fail on tap. Empty for plugin connections, whose
	// grants are per-capability rather than a tier.
	Scope string `json:"scope,omitempty"`
}

// pluginIdent is the identity a per-plugin bus token resolves to: which plugin,
// and the grants it holds — the capabilities it may call, each with optional
// filesystem scoping.
type pluginIdent struct {
	id     string
	caps   map[string]capGrant
	events capspec.EventGrants
}

// capGrant is what a plugin token may do with one capability. fsRoots, when
// non-empty, confines a path-scoped call (fs.*, search.project) to targets
// inside one of these canonical roots; empty means the method carries no path to
// confine (driving, observation, notifications, …).
type capGrant struct {
	fsRoots []string
}

// ScopedIdent is the identity a capability-scoped user token resolves to: a
// human-readable scope name (surfaced in deny errors) and the method patterns
// (exact or `prefix.*`/`*`, matched with event.Matches) the token may call.
// Unlike a plugin ident it carries no filesystem confinement — it is a person's
// credential, tiered by verb, not a sandboxed program's.
type ScopedIdent struct {
	Scope   string
	Methods []string
}

// operator reports whether the ident grants everything — such a token is
// treated exactly like the host token (trusted: may also register providers,
// publish, and pass Authorized for token-guarded HTTP routes).
func (si ScopedIdent) operator() bool {
	for _, m := range si.Methods {
		if m == "*" {
			return true
		}
	}
	return false
}

// Server adapts a broker (events) + router (calls) to HTTP/WebSocket.
type Server struct {
	broker *broker.Broker
	router *router
	extra  map[string]http.HandlerFunc
	token  string // when non-empty, /bus + protected routes require this bearer token

	// Capability-scoped user tokens (tokens.json, minted by `workspacer token
	// create`). Resolved via an injected lookup so the hub decides persistence /
	// live reload and the bus stays a pure policy point. Nil = feature off.
	scopedLookup func(token string) (ScopedIdent, bool)

	// Per-plugin tokens: a connection presenting one is tagged as that plugin and
	// may only call the capabilities it declared. The host token (s.token) is
	// trusted (full access). Registered by the plugin manager.
	ptMu         sync.RWMutex
	pluginTokens map[string]pluginIdent
}

// SetScopedTokenLookup installs the resolver for capability-scoped user tokens.
// It runs at connection handshake (and on Authorized), so a lookup backed by a
// reloading store makes newly minted / revoked tokens take effect on the next
// connection without restarting the hub.
func (s *Server) SetScopedTokenLookup(fn func(token string) (ScopedIdent, bool)) {
	s.scopedLookup = fn
}

// lookupScoped resolves a presented token to a scoped ident, if the feature is
// wired and the token is known.
func (s *Server) lookupScoped(token string) (ScopedIdent, bool) {
	if s.scopedLookup == nil || token == "" {
		return ScopedIdent{}, false
	}
	return s.scopedLookup(token)
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
func (s *Server) RegisterPluginToken(token, pluginID string, grants []capspec.Grant, events capspec.EventGrants) {
	if token == "" {
		return
	}
	set := make(map[string]capGrant, len(grants))
	for _, g := range grants {
		if g.Method == "" {
			continue
		}
		// Fail closed on the drift capspec exists to prevent: a method whose name
		// marks it filesystem-scoped (fs.*, search.*) but that has no PathParam
		// entry would be admitted by authorize() with NO path confinement. Refuse
		// to grant it at all rather than grant it unconfined, and log so the
		// missing spec is visible instead of becoming a silent privilege escape.
		if capspec.MissingSpec(g.Method) {
			log.Printf("[bus] SECURITY: refusing to grant %q to plugin %q — it is named like a filesystem capability but has no internal/capspec.PathParam entry, so it would run unconfined. Add it to capspec (with the params field carrying its path) before granting it.", g.Method, pluginID)
			continue
		}
		set[g.Method] = capGrant{fsRoots: canonRoots(g.FSRoots, pluginID, g.Method)}
	}
	s.ptMu.Lock()
	s.pluginTokens[token] = pluginIdent{id: pluginID, caps: set, events: events}
	s.ptMu.Unlock()
}

// canonRoots canonicalizes grant roots once at registration, DISCARDING any that
// can't confine anything safely: empty, whitespace-only, relative (including a
// "~" prefix, which nobody expands) or unresolvable. Handing "" to a resolver
// would return the daemon's own working directory and silently grant it, which
// is why the empty/relative test lives in canonicalizeRoot rather than being
// left to the walk.
//
// A discard is logged with the plugin and capability that declared it: a broken
// manifest root that merely stops working is a support ticket nobody can read,
// and one that silently widened the grant would be worse.
func canonRoots(roots []string, pluginID, method string) []string {
	if len(roots) == 0 {
		return nil
	}
	out := make([]string, 0, len(roots))
	for _, r := range roots {
		c, ok := canonicalizeRoot(r)
		if !ok {
			log.Printf("[bus] plugin %q capability %q: discarding declared filesystem root %q — it is empty, relative or unresolvable, so it grants nothing. Declare an absolute path (no \"~\").", pluginID, method, r)
			continue
		}
		out = append(out, c)
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

// Authorized reports whether a request carries a token with FULL access: the
// host token, or a scoped token whose grant is operator (`*`). Guarded HTTP
// routes (plugin admin, /remote, /app entry) are operator surface, so view /
// triage tokens do not pass — the /m PWA they pair with is served unguarded
// and the real boundary stays /bus. Always true when no token is configured.
// Accepts either `Authorization: Bearer <token>` or a `?token=<token>` query
// param — browsers can't set headers on a WebSocket handshake, so the query
// form is what the mobile client uses.
func (s *Server) Authorized(r *http.Request) bool {
	if s.token == "" {
		return true
	}
	tok := presentedToken(r)
	if tok == s.token {
		return true
	}
	if si, ok := s.lookupScoped(tok); ok && si.operator() {
		return true
	}
	return false
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

// RegisterLocalIdent is RegisterLocal for a handler that needs to know which
// connection is calling — see [CallerIdentity]. Registering the same method
// through either function replaces the other.
func (s *Server) RegisterLocalIdent(method string, h LocalIdentHandler) {
	s.router.registerLocalIdent(method, h)
}

// TokenFingerprint is the stable, non-secret identity of a bearer token: hex
// SHA-256 of its value. Anything that must remember WHICH credential did
// something records this instead of the token, so a stored fingerprint is never
// a usable secret if the file holding it leaks. Callers that need to test a
// fingerprint against the live token set fingerprint those tokens with this same
// function — one implementation, so the two sides can't drift.
func TokenFingerprint(token string) string {
	if token == "" {
		return ""
	}
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
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

func (s *Server) handleHealth(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	// Subscriber/method counts are internal topology. With no token configured
	// (the loopback default) or an authorized caller, expose them — they're handy
	// for local ops and tests. But once a token guards the bus, an unauthenticated
	// probe (a malicious page hitting loopback, or an unauthorized remote client)
	// gets liveness only, never the counts.
	if s.token != "" && !s.Authorized(r) {
		_ = json.NewEncoder(w).Encode(map[string]any{"status": "ok"})
		return
	}
	_ = json.NewEncoder(w).Encode(map[string]any{
		"status":      "ok",
		"subscribers": s.broker.SubscriberCount(),
		"methods":     s.router.methodCount(),
	})
}

// originAllowed implements the WebSocket same-origin policy for /bus, replacing
// coder/websocket's InsecureSkipVerify (which accepted every Origin, so any web
// page the user visited could open ws://127.0.0.1/bus and drive the control
// plane). The bus is a loopback control plane that, under remote sharing, is also
// reached over Tailscale by a web client the hub itself serves. The policy:
//
//   - No Origin header → allow. Non-browser clients (the Electron main process on
//     the `ws` library, the native mobile client, CLIs, and the busclient used by
//     brain/MCP) don't send Origin. Only a browser's same-origin policy is being
//     enforced here; a native client that reached us at all already has the token.
//   - Origin host == request Host → allow. The same-origin case: the web remote is
//     served BY the hub, so the page's origin host — including a Tailscale
//     hostname or a bare LAN IP:port — equals the Host it dials. Case-insensitive.
//   - Loopback origin (localhost / 127.0.0.0/8 / ::1, any port) → allow. Covers a
//     local dev renderer served on a different port; a remote attacker's page is
//     never served from the victim's own loopback.
//   - Anything else (a cross-site browser origin) → reject. This is the malicious-
//     page / DNS-rebinding vector the finding flags.
func originAllowed(r *http.Request) bool {
	origin := r.Header.Get("Origin")
	if origin == "" {
		return true // non-browser client — no browser same-origin policy to enforce
	}
	u, err := url.Parse(origin)
	if err != nil || u.Host == "" {
		return false // malformed / opaque ("null") Origin — fail closed
	}
	// Same-origin: the Origin's host (host[:port]) equals the Host the client
	// dialed. Browsers set Origin to scheme://host[:port]; r.Host is host[:port].
	// Equal ⇒ the page was served from this very endpoint (hub-served remote UI).
	if strings.EqualFold(u.Host, r.Host) {
		// r.Host is the browser-supplied Host header, so same-origin alone does not
		// prove where the socket landed. A loopback Host is always safe (the
		// attacker's page is never served from the victim's own loopback).
		if isLoopbackHost(u.Hostname()) {
			return true
		}
		// Non-loopback same-origin: reject if the connection actually terminated at
		// loopback. That mismatch — public Host header, loopback socket — is the DNS
		// rebinding vector: the browser still reports the name it was served from
		// while the rebound A record points the socket at 127.0.0.1.
		if la, ok := r.Context().Value(http.LocalAddrContextKey).(net.Addr); ok {
			if host, _, err := net.SplitHostPort(la.String()); err == nil {
				if ip := net.ParseIP(host); ip != nil && ip.IsLoopback() {
					return false
				}
			}
		}
		return true
	}
	// Loopback origins are always local to the user's machine, so a dev renderer on
	// another localhost port is fine while a remote page never qualifies.
	return isLoopbackHost(u.Hostname())
}

func isLoopbackHost(host string) bool {
	if strings.EqualFold(host, "localhost") {
		return true
	}
	if ip := net.ParseIP(host); ip != nil {
		return ip.IsLoopback()
	}
	return false
}

func (s *Server) handleBus(w http.ResponseWriter, r *http.Request) {
	// Reject cross-site browser origins before doing any auth work. A non-browser
	// client (Electron main, mobile native, brain/MCP busclient) sends no Origin
	// and passes; a page served by the hub itself is same-origin and passes.
	if !originAllowed(r) {
		http.Error(w, "forbidden origin", http.StatusForbidden)
		return
	}
	// Classify the connection by the token it presents:
	//   - a registered per-plugin token → that plugin, restricted to its caps
	//   - the host token (or no host token configured) → trusted, full access
	//   - a scoped user token (tokens.json) → its tier's method allowlist;
	//     an operator-tier token is trusted, exactly like the host token
	//   - anything else → rejected
	tok := presentedToken(r)
	var trusted bool
	var caps map[string]capGrant
	var pluginID string
	var events capspec.EventGrants
	var scope string
	var scopeMethods []string
	if pi, ok := s.lookupPluginToken(tok); ok {
		caps, pluginID, events = pi.caps, pi.id, pi.events
	} else if s.token == "" || tok == s.token {
		trusted = true
	} else if si, ok := s.lookupScoped(tok); ok {
		if si.operator() {
			trusted = true
		} else {
			scope, scopeMethods = si.Scope, si.Methods
			if scopeMethods == nil {
				// A record with no grants must still be a real deny-all identity,
				// not accidentally mistaken for "unscoped" downstream.
				scopeMethods = []string{}
			}
		}
	} else {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	// InsecureSkipVerify is intentional: origin is already enforced by
	// originAllowed above (a testable policy that must allow no-Origin native
	// clients and loopback dev renderers, which OriginPatterns can't express), so
	// we take over the check rather than let the library re-run its own.
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
	cn := &conn{
		ws: ws, ctx: ctx, trusted: trusted, caps: caps, pluginID: pluginID,
		emits: events.Emits, consumes: events.Consumes, provides: events.Provides,
		scope: scope, scopeMethods: scopeMethods, tokenID: TokenFingerprint(tok),
	}
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
			// Enforce the consume grant even when the plugin subscribed more
			// broadly (e.g. "*") — the manifest's `consumes` is the ceiling on
			// what it can ever receive, not just what it asked for.
			if !cn.mayConsume(ev.Type) {
				continue
			}
			if err := cn.send(Frame{Op: "event", Event: &ev}); err != nil {
				return
			}
			// The broker drops past this subscriber's capacity. For a discrete
			// event that is the design; for a PTY byte stream the dropped chunk
			// silently corrupts the client's terminal — xterm renders garbage
			// and neither side knows. Tell it, so it re-attaches and takes a
			// fresh screen replay.
			for _, topic := range sub.TakeDesyncs() {
				sid := strings.TrimPrefix(topic, broker.StreamTopicPrefix)
				data, _ := json.Marshal(map[string]string{"sessionId": sid})
				desync := event.Envelope{
					Type:   "pty.desync",
					Source: "hub",
					Time:   time.Now(),
					Data:   data,
				}
				if !cn.mayConsume(desync.Type) {
					continue
				}
				if err := cn.send(Frame{Op: "event", Event: &desync}); err != nil {
					return
				}
			}
		}
	}()

	_ = cn.send(cn.helloFrame())

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
			if len(f.Topics) > maxFrameTopics {
				_ = cn.send(Frame{Op: "error", Error: tooManyTopics("subscribe", len(f.Topics))})
				continue
			}
			sub.AddTopics(f.Topics...)
			_ = cn.send(Frame{Op: "subscribed", Topics: sub.Topics()})
		case "unsubscribe":
			if len(f.Topics) > maxFrameTopics {
				_ = cn.send(Frame{Op: "error", Error: tooManyTopics("unsubscribe", len(f.Topics))})
				continue
			}
			sub.RemoveTopics(f.Topics...)
			_ = cn.send(Frame{Op: "unsubscribed", Topics: sub.Topics()})
		case "publish":
			if f.Event == nil {
				_ = cn.send(Frame{Op: "error", Error: "publish missing event"})
				continue
			}
			// A plugin may publish only the event types its manifest declared in
			// `emits`. This is the gate that stops an untrusted plugin from, e.g.,
			// publishing a `command.*` event to drive the app without holding the
			// capability — commands must go through `call`, or be an explicitly
			// granted emit.
			if !cn.mayPublish(f.Event.Type) {
				if cn.scopeMethods != nil {
					_ = cn.send(Frame{Op: "error", Error: fmt.Sprintf("not authorized: publishing events is outside this token's %q scope", cn.scope)})
				} else {
					_ = cn.send(Frame{Op: "error", Error: "plugin not authorized to publish event " + f.Event.Type})
				}
				continue
			}
			s.broker.Publish(*f.Event)
		case "register":
			// A plugin may register as a provider only for methods its manifest
			// declared in `provides`; disallowed ones are dropped, and the ack
			// reflects what was actually registered.
			accepted := s.router.register(cn, f.Methods)
			_ = cn.send(Frame{Op: "registered", Methods: accepted})
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

// tooManyTopics renders the rejection so a client can see the ceiling it hit
// rather than guess why its subscription didn't take.
func tooManyTopics(op string, n int) string {
	return fmt.Sprintf("%s carries %d topics, over the %d limit for one frame", op, n, maxFrameTopics)
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
	// Scoped user token (tokens.json): the tier name (for deny errors) and the
	// method patterns it may call. scopeMethods non-nil marks the conn as
	// token-scoped: it may subscribe to and receive every event (view includes
	// streams) but may not publish or register as a provider, and may call only
	// matching methods. Nil on trusted and plugin conns.
	scope        string
	scopeMethods []string
	// Event-side grants (empty for a trusted conn, which bypasses these): which
	// event types this plugin may publish / receive, and which capability methods
	// it may register as a provider of. Patterns are matched with event.Matches.
	emits    []string
	consumes []string
	provides []string
	// tokenID fingerprints the credential this connection presented, for local
	// handlers that persist something on its behalf (see [CallerIdentity]). Never
	// the token itself: nothing downstream should be able to replay it.
	tokenID string

	// Frames this connection asked the hub to deliver to OTHER connections,
	// drained in order by one goroutine started on first use — see [conn.forward].
	fwdOnce sync.Once
	fwd     chan forwardTask
}

// forwardTask is one frame a connection asked the hub to deliver to another
// connection, with what to do if that delivery fails.
type forwardTask struct {
	target *conn
	frame  Frame
	onErr  func()
}

// forward hands a frame to this connection's dispatcher for delivery to another
// connection: off the read loop, but in the order the read loop produced. Both
// halves matter, and each was a bug on its own.
//
// Off the read loop, because conn.send serializes on the TARGET's write mutex.
// Sent inline, one peer that has stopped draining head-of-line blocks every
// subsequent frame this client sends — on a socket it has nothing to do with.
//
// In order, because a goroutine per forward is not ordered at all: Go schedules
// them as it pleases, and successive calls from one connection routinely
// inverted in practice. That is not abstract here — the web client fires
// sessions.terminalInput per keystroke/chunk without awaiting the result, so
// reordering scrambles what is typed into a PTY.
//
// The cost of the ordering is that a call to a wedged provider delays this
// caller's LATER calls (a failing write gives up after writeTimeout, so the
// queue drains rather than wedges). Returns false when the queue is full, which
// only happens against a peer that has stopped reading entirely; the caller
// fails the call rather than buffer without limit.
func (cn *conn) forward(t forwardTask) bool {
	cn.fwdOnce.Do(func() {
		cn.fwd = make(chan forwardTask, forwardQueueDepth)
		go cn.dispatchForwards()
	})
	select {
	case cn.fwd <- t:
		return true
	default:
		return false
	}
}

// dispatchForwards delivers this connection's queued forwards one at a time,
// which is what makes them ordered. It ends with the connection: anything still
// queued when the caller goes away is already being cleaned up by dropConn,
// which fails or drops the pending calls those frames belonged to.
func (cn *conn) dispatchForwards() {
	for {
		select {
		case <-cn.ctx.Done():
			return
		case t := <-cn.fwd:
			if err := t.target.send(t.frame); err != nil && t.onErr != nil {
				t.onErr()
			}
		}
	}
}

// identity is what a local handler sees of its caller. Trusted conns report the
// "operator" tier for the same reason helloFrame does — a host token and an
// operator token are the same authority, and the latter is promoted to trusted
// at the handshake, so the tier name isn't otherwise recoverable.
func (cn *conn) identity() CallerIdentity {
	id := CallerIdentity{
		Trusted:  cn.trusted,
		Scope:    cn.scope,
		PluginID: cn.pluginID,
		TokenID:  cn.tokenID,
	}
	if cn.trusted {
		id.Scope = "operator"
	}
	return id
}

// helloFrame is the greeting a client gets the moment its token resolved. It
// tells a scoped client what tier it holds so the UI can gate itself up front —
// /m greys out spawning on a triage token instead of offering a button that
// dies on tap. Operator and host tokens report the same "operator" ceiling:
// they are the same authority, and an operator record is promoted to trusted at
// the handshake, so the tier name is not otherwise recoverable here.
func (cn *conn) helloFrame() Frame {
	f := Frame{Op: "hello"}
	switch {
	case cn.trusted:
		f.Scope, f.Methods = "operator", []string{"*"}
	case cn.scopeMethods != nil:
		f.Scope, f.Methods = cn.scope, cn.scopeMethods
	}
	return f
}

// mayPublish reports whether this connection may publish an event of the given
// type. Trusted conns publish anything; a plugin may publish only types matched
// by its manifest's `emits`.
func (cn *conn) mayPublish(typ string) bool {
	return cn.trusted || event.MatchesAny(cn.emits, typ)
}

// mayConsume reports whether an event of the given type may be delivered to this
// connection. Trusted conns receive everything they subscribed to; a scoped
// user token likewise (event/stream subscriptions are part of even the view
// tier); a plugin only receives types matched by its manifest's `consumes`, so
// a broad `subscribe` can never widen its reach past what it declared.
func (cn *conn) mayConsume(typ string) bool {
	return cn.trusted || cn.scopeMethods != nil || event.MatchesAny(cn.consumes, typ)
}

// mayProvide reports whether this connection may register as the provider of a
// capability method. Trusted conns (the host) provide the built-in capabilities;
// a plugin may register only methods matched by its manifest's `provides`.
func (cn *conn) mayProvide(method string) bool {
	return cn.trusted || event.MatchesAny(cn.provides, method)
}

// mayCall reports whether this connection is allowed to invoke method at all
// (the verb check). Trusted connections (the host / MCP facade) may call
// anything; a scoped user token may call only methods matching its tier's
// patterns; a plugin may call only the capabilities it was granted. Argument
// scoping (which paths) is a separate step — see authorize.
func (cn *conn) mayCall(method string) bool {
	if cn.trusted {
		return true
	}
	if cn.scopeMethods != nil {
		return event.MatchesAny(cn.scopeMethods, method)
	}
	_, ok := cn.caps[method]
	return ok
}

// callDenied renders the error for a call mayCall refused, naming what the
// caller is (its scope or plugin identity) so the fix is obvious client-side.
func (cn *conn) callDenied(method string) string {
	if cn.scopeMethods != nil {
		return fmt.Sprintf("not authorized: method %q is outside this token's %q scope (mint a broader token with `workspacer token create`)", method, cn.scope)
	}
	return "plugin not authorized for capability " + method
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
	if cn.scopeMethods != nil {
		// Scoped user tokens are tiered by verb only (mayCall) — they are a
		// person's credential, not a sandboxed program's, so no path confinement.
		return nil
	}
	g, ok := cn.caps[method]
	if !ok {
		return fmt.Errorf("plugin not authorized for capability %s", method)
	}
	field, scoped := capspec.IsPathScoped(method)
	if !scoped {
		// Defense in depth: RegisterPluginToken already refuses to grant a method
		// that looks path-bearing but was never classified, so mayCall should have
		// denied it before we got here. If one slips through anyway, deny rather
		// than let an unscoped filesystem method run with no containment. The test
		// is MissingSpec, not LooksPathBearing — a method capspec deliberately
		// leaves unconfined (with its reason on the record) is allowed through.
		if capspec.MissingSpec(method) {
			return fmt.Errorf("%s: named like a filesystem capability but has no capspec entry; denied to avoid running unconfined", method)
		}
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
	switch {
	case errors.Is(err, errSecretPath):
		// The SECRET arm deliberately says nothing about the path: it reaches a
		// remote caller, and confirming that a denied path hit something worth
		// protecting is a probe primitive. Same wording as the brain's
		// assertPathAllowed. The containment arm below keeps its own, path-
		// echoing message — a plugin's grant scope is its own install-time
		// consented data, so naming it back is not a disclosure.
		return fmt.Errorf("%s: path is outside the allowed workspace (agent cwds + config stores)", method)
	case err != nil:
		return fmt.Errorf("%s: cannot resolve %q: %w", method, target, err)
	case !within:
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
