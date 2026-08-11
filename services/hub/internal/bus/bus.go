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
	"sync/atomic"
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
	// Live connections that presented each plugin token. UnregisterPluginToken
	// only ever removed the token from the map above, which decides the
	// HANDSHAKE — conn.caps is a snapshot taken once at accept time and nothing
	// re-consulted it, so revocation was a no-op on a socket that was already
	// open. A pane token is the only way a plugin gets ${agentCwd} roots (the
	// static token deliberately gets none), so a plugin that held one pane socket
	// open kept fs.read/fs.write inside that agent's cwd after the pane closed,
	// after it was disabled, and after it was removed. The manager calls that
	// state "an unrevocable grant leak"; this map is what makes revoking real.
	pluginConns map[string]map[*conn]struct{}

	// trustedHosts are Host/Origin names this hub is deliberately reached by
	// through a REVERSE PROXY that terminates elsewhere and forwards to our
	// loopback socket. `tailscale serve` — the app's one-tap "HTTPS via
	// Tailscale" toggle, and the only way the mobile PWA gets the secure
	// context Web Push needs — is exactly that: it presents
	// `Host: <node>.ts.net` on a connection that lands on 127.0.0.1, which is
	// byte-for-byte the DNS-rebinding shape requireHost/originAllowed refuse.
	// The guard is right and cannot tell the two apart from the request alone,
	// so the operator names the proxy's hostname instead (hub --trusted-host,
	// set by the desktop when it enables the toggle). Empty = no exemption,
	// i.e. today's shape-only rule.
	trustedHosts map[string]struct{}
}

// SetTrustedHosts declares the hostnames a reverse proxy in front of this hub
// presents. Names are compared case-insensitively, without their port. Passing
// none clears the list.
//
// This is a deliberate, operator-supplied exemption from the rebinding shape:
// naming a host says "requests claiming to be this name, on any socket, are
// mine". It must never be defaulted to anything, and in particular never to a
// wildcard — a `*` here would re-open every route to any page that can resolve
// a name to 127.0.0.1.
func (s *Server) SetTrustedHosts(names []string) {
	set := make(map[string]struct{}, len(names))
	for _, n := range names {
		n = strings.ToLower(strings.TrimSpace(hostWithoutPort(n)))
		if n == "" || n == "*" {
			continue
		}
		set[n] = struct{}{}
	}
	s.trustedHosts = set
}

// isTrustedHost reports whether name (already port-stripped) was declared with
// SetTrustedHosts.
func (s *Server) isTrustedHost(name string) bool {
	if len(s.trustedHosts) == 0 {
		return false
	}
	_, ok := s.trustedHosts[strings.ToLower(name)]
	return ok
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
	return &Server{
		broker: b, router: newRouter(), extra: map[string]http.HandlerFunc{},
		pluginTokens: map[string]pluginIdent{},
		pluginConns:  map[string]map[*conn]struct{}{},
	}
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

// UnregisterPluginToken drops a plugin token (on unload/replace) AND revokes it
// on every connection that already presented it.
//
// Dropping it from pluginTokens alone governs the next handshake and nothing
// else: conn.caps is a snapshot taken at accept time, so a socket opened one
// millisecond earlier kept its grants for as long as it stayed open. That made
// every caller of this function — plugin unload, plugin removal, and
// Manager.revokePaneTokensFor, whose own comment says it exists "so a closed
// plugin's panes can't keep calling" — advisory rather than enforcing.
//
// Both halves are needed. `revoked` is what makes the NEXT call on an in-flight
// connection fail even before the close lands, and CloseNow is what stops the
// connection from sitting there consuming events.
func (s *Server) UnregisterPluginToken(token string) {
	if token == "" {
		return
	}
	s.ptMu.Lock()
	delete(s.pluginTokens, token)
	conns := s.pluginConns[token]
	delete(s.pluginConns, token)
	s.ptMu.Unlock()
	for cn := range conns {
		cn.revoked.Store(true)
		_ = cn.ws.CloseNow()
	}
}

// trackPluginConn registers a live plugin connection under the token it
// presented, so UnregisterPluginToken can reach it. It reports whether the token
// is STILL registered; false means the caller must treat the connection as
// revoked before serving anything on it.
//
// That return value closes a real race, not a theoretical one. The handshake
// resolves the token (lookupPluginToken) BEFORE websocket.Accept, and only
// registers the connection here afterwards. UnregisterPluginToken snapshots
// pluginConns under the same lock and closes exactly what it finds — so a dial
// whose lookup ran before the delete and whose track runs after it is in NEITHER
// set: never closed, never flagged, and conn.caps is a snapshot taken at accept,
// so it keeps its full ${agentCwd} grants for the life of the process. A plugin
// sidecar holding its .bus-token and reconnecting in a loop wins that race
// trivially, which means the grant survives disable, reload and uninstall — the
// exact state pane close and plugin removal call this function to prevent.
//
// Re-checking the token HERE, under the same mutex UnregisterPluginToken takes,
// serializes the two: either the delete lands first and this returns false, or
// this lands first and the delete finds the connection. There is no third
// interleaving.
func (s *Server) trackPluginConn(token string, cn *conn) bool {
	s.ptMu.Lock()
	defer s.ptMu.Unlock()
	if _, live := s.pluginTokens[token]; !live {
		return false
	}
	set := s.pluginConns[token]
	if set == nil {
		set = map[*conn]struct{}{}
		s.pluginConns[token] = set
	}
	set[cn] = struct{}{}
	return true
}

// untrackPluginConn removes a connection when it goes away on its own.
func (s *Server) untrackPluginConn(token string, cn *conn) {
	s.ptMu.Lock()
	defer s.ptMu.Unlock()
	if set := s.pluginConns[token]; set != nil {
		delete(set, cn)
		if len(set) == 0 {
			delete(s.pluginConns, token)
		}
	}
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

// AuthorizedForPlugin reports whether a request may see ONE plugin's own
// non-public state: true for a fully-authorized caller (the host / an operator
// token, per Authorized), and true for a caller presenting that plugin's own bus
// credential — its per-plugin token or a pane token minted for it, which is what
// the host injects into that plugin's webview URL as ?busToken=.
//
// It exists because /plugins/ui/<id>/ serves a plugin's HTML with
// window.__WKS_SETTINGS__ = that plugin's merged setting values inlined, and the
// route is unguarded by design (a <script>/webview URL cannot carry the host
// token). Secrets are already redacted there, but the NON-secret half —
// endpoints, org/repo names, absolute paths — is exactly what
// plugin.settings.changed is TopicHostOnly for, and what the guard()ed GET
// /plugins/settings refuses; an anonymous GET with any Host header was reading
// it out of an HTML document. A plugin's own settings are the plugin's own
// business (its sidecar receives them in plaintext in WKS_SETTINGS), so the
// credential that identifies the plugin is the right key, and nothing weaker is.
//
// Always true when no token is configured — the loopback-only default, where
// Authorized already says the same thing.
func (s *Server) AuthorizedForPlugin(r *http.Request, pluginID string) bool {
	if s.Authorized(r) {
		return true
	}
	if pluginID == "" {
		return false
	}
	for _, tok := range presentedPluginTokens(r) {
		if pi, ok := s.lookupPluginToken(tok); ok && pi.id == pluginID {
			return true
		}
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

// presentedPluginTokens lists the credentials a plugin webview can be carrying.
// The host injects the per-plugin (or pane) token as ?busToken=; sdk.js then
// re-presents the same value to /bus as ?token=, so both spellings are read.
func presentedPluginTokens(r *http.Request) []string {
	q := r.URL.Query()
	out := make([]string, 0, 3)
	for _, tok := range []string{presentedToken(r), q.Get("busToken")} {
		if tok != "" {
			out = append(out, tok)
		}
	}
	return out
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

// Handler returns the routed HTTP handler (/bus WebSocket, /health JSON, extras),
// wrapped in the Host pin — see requireHost.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/bus", s.handleBus)
	mux.HandleFunc("/health", s.handleHealth)
	for path, h := range s.extra {
		mux.HandleFunc(path, h)
	}
	return s.requireHost(mux)
}

// requireHost refuses the DNS-REBINDING SHAPE on every route the hub serves: a
// non-loopback Host header on a request whose socket terminated at loopback.
//
// This is the hub's copy of a rule that had two copies and an absentee.
// claudemon pins Host on both of its routers (AllowedHosts::permits, api.rs);
// the MCP facade wraps requireHost around its whole mux and names claudemon's
// guard as its TWIN. The hub — the one server that is deliberately reachable
// beyond loopback — had no Host pin at all: only /bus was rebinding-aware,
// inside originAllowed, and that left every other route (the plugin manifest
// list, the plugin UI documents, /m, the static assets) readable same-origin by
// a page whose own name resolves to 127.0.0.1. Measured before this existed:
// `Host: evil.example.com` got 403 from /bus and 200 + the full manifest from
// /plugins on the same hub. Rebinding is the one browser path where the absence
// of CORS headers stops protecting the bytes, because after the rebind there is
// no cross-origin read to block.
//
// The rule is SHAPED, not an allowlist, and it has to be: the hub's whole point
// under remote sharing is to be reached by a name this process never learns —
// a Tailscale MagicDNS name, a LAN hostname, whatever the operator typed. So
// instead of enumerating hosts, it refuses the one combination that cannot
// legitimately occur: a public/foreign Host arriving on the loopback listener.
// That is precisely the predicate originAllowed already applies to same-origin
// WebSocket upgrades, lifted to the whole mux.
//
//   - No Host at all → allow. HTTP/1.0 and some local probes omit it, and it is
//     not a header an attacker gains by dropping. Both twins do the same.
//   - Loopback Host (localhost / 127.0.0.0/8 / ::1) → allow.
//   - Host equal to the address the socket actually landed on → allow (an
//     operator dialing the tailnet IP directly).
//   - Anything else → allowed only if the socket did NOT land on loopback. A
//     shared bind reached by its own name is the supported deployment; the same
//     name arriving on 127.0.0.1 is a rebind.
//
// RESIDUAL, recorded rather than papered over: on a deliberately shared bind
// (remote-share, 0.0.0.0), a foreign Host on a NON-loopback socket is still
// allowed, because refusing it would mean enumerating the names this hub may be
// called by — which it cannot know. An attacker who rebinds a name to the
// victim's tailnet address, having first learned that address, is not closed by
// this. /bus's originAllowed makes the same trade for the same reason.
func (s *Server) requireHost(h http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !s.hostAllowed(r) {
			http.Error(w, "host not allowed", http.StatusForbidden)
			return
		}
		h.ServeHTTP(w, r)
	})
}

// hostAllowed is requireHost's predicate, split out so the test can drive it
// directly with a synthesized local address.
func (s *Server) hostAllowed(r *http.Request) bool {
	if r.Host == "" {
		return true
	}
	name := hostWithoutPort(r.Host)
	if isLoopbackHost(name) {
		return true
	}
	// A proxy hostname the operator declared (see SetTrustedHosts). Checked
	// before the socket shape, because the whole point is that the socket
	// landed on loopback — the proxy is on this machine.
	if s.isTrustedHost(name) {
		return true
	}
	local, known := localAddrOf(r)
	if !known {
		// Nothing to compare against (an in-process handler call, a test
		// recorder): the rebinding shape is defined by the socket, and with no
		// socket there is no shape to refuse.
		return true
	}
	if name == local.String() {
		return true
	}
	return !local.IsLoopback()
}

// localAddrOf reports the IP the request's connection actually terminated at.
func localAddrOf(r *http.Request) (net.IP, bool) {
	la, ok := r.Context().Value(http.LocalAddrContextKey).(net.Addr)
	if !ok || la == nil {
		return nil, false
	}
	host, _, err := net.SplitHostPort(la.String())
	if err != nil {
		host = la.String()
	}
	ip := net.ParseIP(host)
	return ip, ip != nil
}

// hostWithoutPort strips a trailing `:port` from a Host/authority, handling
// bracketed IPv6 (`[::1]:7895` → `::1`). Mirrors cmd/mcp's helper of the same
// name and claudemon's `host_without_port`.
func hostWithoutPort(h string) string {
	if rest, ok := strings.CutPrefix(h, "["); ok {
		if name, _, found := strings.Cut(rest, "]"); found {
			return name
		}
		return h
	}
	if name, _, found := strings.Cut(h, ":"); found {
		return name
	}
	return h
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
		// The NAMES, not just the count. A bare count cannot answer the only
		// question a client or operator actually has — "is the plane I am about
		// to call provided?" — so an entire capability plane could die with the
		// count as the only trace, and nothing compares a count to anything.
		"methodNames": s.router.providedMethods(),
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
func (s *Server) originAllowed(r *http.Request) bool {
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
		// A declared reverse-proxy hostname (see SetTrustedHosts) is the one
		// legitimate producer of the mismatch below: `tailscale serve`
		// terminates TLS for <node>.ts.net and forwards to our loopback socket,
		// so the page IS same-origin and the socket IS loopback. Without this,
		// the app's own one-tap HTTPS toggle 403s its own /bus upgrade.
		if s.isTrustedHost(u.Hostname()) {
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

// scopedRevalidateInterval is how often a live scoped-token connection
// re-presents its credential to the store. Small enough that a stolen phone is
// cut off in seconds; large enough that the cost is one mtime-gated map lookup
// per connection per tick (authtoken.Store only re-reads the file when it
// changed).
// Stored atomically: the test shortens it while revalidation goroutines from
// EARLIER tests are still ticking, so a plain package var is a real data race
// (`go test -race ./internal/bus` reported it on every full-package run).
var scopedRevalidateNanos atomic.Int64

func init() { scopedRevalidateNanos.Store(int64(5 * time.Second)) }

func scopedRevalidateInterval() time.Duration {
	return time.Duration(scopedRevalidateNanos.Load())
}

// revalidateScoped re-checks a live connection's scoped token against the store
// and CLOSES the socket when it stops resolving, or resolves to a different
// tier.
//
// Revocation was a control-plane act the event plane never heard about.
// `workspacer token revoke` rewrites tokens.json and the store re-reads it, so a
// NEW dial with that token is correctly 401'd — the code's own comment says
// revoking "takes effect on the next connection". The socket already open was
// untouched: handshake classification happens once (scopeMethods is a snapshot),
// nothing re-consulted the store, nothing set revoked, nothing closed the
// socket. A revoked phone kept receiving the whole event firehose and kept
// making capability calls for the life of the hub process — and an
// operator-scoped token is promoted to `trusted` at handshake, so a revoked
// operator socket kept FULL host authority.
//
// This is exactly the hole that was already found and fixed for PLUGIN tokens
// (UnregisterPluginToken sets revoked AND calls CloseNow, pinned by
// TestRevocationClosesTheConnectionAndStopsEventDelivery). Scoped user tokens —
// the phone / web-remote tier, the one credential a user is actually expected to
// revoke — never got the same treatment, which left revocation advisory for the
// only credential class it exists for.
//
// Polling rather than a push from the store, deliberately: the store is a
// read-through cache over a FILE that any process may rewrite (the CLI does,
// out-of-process), so there is no event to subscribe to. internal/push made the
// same call for the same reason — it re-checks HasFingerprint on every
// notification because a subscription outlives the connection that made it.
func (s *Server) revalidateScoped(ctx context.Context, cn *conn, tok, authScope string) {
	t := time.NewTicker(scopedRevalidateInterval())
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			si, ok := s.lookupScoped(tok)
			if ok && si.Scope == authScope {
				continue
			}
			// Both halves, exactly as UnregisterPluginToken applies them: the
			// flag so anything already read off the wire is denied even before
			// the close lands, and the close so the socket does not sit there
			// consuming events.
			cn.revoked.Store(true)
			_ = cn.ws.CloseNow()
			if ok {
				log.Printf("[bus] scoped token %s: tier changed %q -> %q while connected; connection closed", cn.tokenID, authScope, si.Scope)
			} else {
				log.Printf("[bus] scoped token %s revoked; connection closed", cn.tokenID)
			}
			return
		}
	}
}

func (s *Server) handleBus(w http.ResponseWriter, r *http.Request) {
	// Reject cross-site browser origins before doing any auth work. A non-browser
	// client (Electron main, mobile native, brain/MCP busclient) sends no Origin
	// and passes; a page served by the hub itself is same-origin and passes.
	if !s.originAllowed(r) {
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
	// viaScoped marks a connection authenticated from tokens.json, so its
	// credential can be RE-checked while the socket is open — see
	// revalidateScoped. authScope is the tier it presented, so a downgrade is a
	// revocation too.
	var viaScoped bool
	var authScope string
	if pi, ok := s.lookupPluginToken(tok); ok {
		caps, pluginID, events = pi.caps, pi.id, pi.events
	} else if s.token == "" || tok == s.token {
		trusted = true
	} else if si, ok := s.lookupScoped(tok); ok {
		viaScoped, authScope = true, si.Scope
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
	if pluginID != "" {
		// Tracked under the raw token so revocation can find it. The token itself
		// is not stored on the conn (tokenID is a fingerprint, deliberately);
		// only this map holds it, and only for the life of the socket.
		if !s.trackPluginConn(tok, cn) {
			// Revoked between the handshake's lookup and this registration. Both
			// halves, exactly as UnregisterPluginToken applies them: the flag so
			// anything already read off the wire is denied, and the close so the
			// socket does not sit there consuming events.
			cn.revoked.Store(true)
			_ = ws.CloseNow()
			return
		}
		defer s.untrackPluginConn(tok, cn)
	}
	if viaScoped {
		go s.revalidateScoped(ctx, cn, tok, authScope)
	}
	defer cn.ws.CloseNow()

	// Filtered at ENQUEUE, not at write. See broker.SubscribeFiltered: checking
	// the consume grant in the writer goroutine below meant a denied topic was
	// still matched, still took a slot in this connection's channel, and — when
	// that channel filled — was still recorded as a drop against it, which the
	// desync path then reported back naming the sessionId of a stream this
	// connection may not consume. The writer-side check stays as well: topics can
	// change under a live subscription, and two cheap checks are worth less than
	// one authorization applied in only one place.
	sub := s.broker.SubscribeFiltered(nil, cn.mayConsume)
	defer s.broker.Unsubscribe(sub)
	defer s.router.dropConn(cn) // unregister provider + fail outstanding calls

	// Writer goroutine: pump matched events to this client. Blocking here (a
	// slow TCP client) only backs up this subscriber's buffer — the broker
	// drops past capacity, so other clients and publishers are unaffected.
	go cn.pumpEvents(sub, cn.send)

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

// pumpEvents drains sub onto this connection until the subscription closes or
// the socket fails. `send` is a parameter, not cn.send inlined, because the two
// authorization checks below are the LAST layer and a test has to be able to
// watch what they let through — see delivery_layers_test.go.
//
// THE SECOND LAYER, kept deliberately. Since SubscribeFiltered, mayConsume is
// already applied at ENQUEUE (bus.go's `sub := s.broker.SubscribeFiltered(nil,
// cn.mayConsume)`), which is what stops a refused topic costing this connection
// a buffer slot and leaving a drop record to leak. That makes the two checks
// here redundant on the happy path — and they stay anyway, for one reason that
// is not a slogan: the admission filter is a snapshot of an authorization that
// CHANGES UNDER A LIVE SUBSCRIPTION. `revoked` is set on an already-open socket
// (UnregisterPluginToken), and an event admitted microseconds before that flag
// landed is sitting in sub.C when it does. The enqueue filter cannot un-admit
// it; this check refuses to write it.
//
// The desync check is the same argument with a sharper edge, because the desync
// frame is SYNTHESISED HERE and so was never offered to the admission filter at
// all. sub.TakeDesyncs() reports drops by stream topic; turning that into a
// pty.desync naming the sessionId is a fresh publish to this connection, and
// pty.desync is a guarded topic in its own right. Without this check the drop
// bookkeeping of a stream is delivered to a connection that may not consume the
// stream — which is exactly the leak SubscribeFiltered was added to close,
// re-entering through the door on the other side of it.
func (cn *conn) pumpEvents(sub *broker.Subscription, send func(Frame) error) {
	for ev := range sub.C {
		ev := ev
		// Enforce the consume grant even when the plugin subscribed more
		// broadly (e.g. "*") — the manifest's `consumes` is the ceiling on
		// what it can ever receive, not just what it asked for.
		if !cn.mayConsume(ev.Type) {
			continue
		}
		if err := send(Frame{Op: "event", Event: &ev}); err != nil {
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
			if err := send(Frame{Op: "event", Event: &desync}); err != nil {
				return
			}
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
	// Set by UnregisterPluginToken when the credential this connection presented
	// is revoked. Checked by mayCall, so the very next frame on an already-open
	// socket is refused rather than answered from the handshake-time snapshot.
	revoked atomic.Bool

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
// by its manifest's `emits`, and NOBODY but a trusted conn may publish a topic
// capspec classifies — those are host state.
//
// The manifest used to be the whole answer, and a manifest is a statement about
// what a plugin WANTS to emit, not about who OWNS the topic. Two proven chains
// came through that gap, both from a plugin holding zero capabilities:
//
//   - `emits: ["layout.changed"]` publishes a layout document carrying
//     skipPermissions, permissionMode, profileId and mcpItemIds — the exact four
//     fields layout.SetAs scrubs from a non-trusted writer because "they stop
//     being description on the desktop's next launch and become arguments to a
//     spawn". The scrub lives on the CALL; every client adopts the BROADCAST.
//     Publishing the document directly skips the service that scrubs it, and a
//     publisher-chosen `version: 999999` also wins every later comparison, so
//     genuine layout.set broadcasts are ignored from then on.
//   - `emits: ["agent.snapshot"]` drives internal/push: the Manager consumes
//     that topic in-process and fires the phone's "needs you" Web Push on the
//     un-blocked→blocked edge, titled from the payload's own liveCwd. A forged
//     snapshot puts attacker-authored text on the lock screen deep-linked to a
//     REAL session, and the same write moves push's state machine past the edge,
//     so the GENUINE approval prompt for that session is never notified.
//
// Both are "a trusted in-hub component treats this topic as authoritative
// input". Ownership is not a per-consumer question, so it is not enforced
// per-consumer. Plugin-defined topics (example.clock.tick, the rules engine's
// command.*) are unclassified and stay manifest-gated: nothing in the hub reads
// them as host state.
func (cn *conn) mayPublish(typ string) bool {
	if cn.revoked.Load() {
		return false
	}
	if cn.trusted {
		return true
	}
	if capspec.EventTopicIsHostOwned(typ) {
		return false
	}
	return event.MatchesAny(cn.emits, typ)
}

// mayConsume reports whether an event of the given type may be delivered to this
// connection. Trusted conns receive everything they subscribed to; a scoped user
// token likewise (event/stream subscriptions are part of even the view tier)
// EXCEPT for topics that carry a capability's output, which require that
// capability; a plugin only receives types matched by its manifest's `consumes`,
// so a broad `subscribe` can never widen its reach past what it declared.
//
// The middle clause used to be unconditional, and that made the two
// authorization planes disagree about the same credential. The capability plane
// refuses `sessions.attachTerminal` to a `view` token — sensitive:true, in
// neither scoped tier — while this function delivered `pty.bytes.<id>` to it
// anyway: the session's raw PTY bytes, ring-buffer replay included, i.e. exactly
// what the refused method produces. terminals.* is in neither scoped tier at
// all, so the event plane was the ONLY door onto a terminal's screen and it was
// open. Neither half was wrong alone; the composition was.
//
// capspec.EventTopicSpec owns the registry, next to the classification of the
// methods it names, and its DEFAULT IS CLOSED. The scoped arm used to end in
// `return true`, which made the table a two-row denylist governing a
// twenty-five-topic plane: a matrix run against the real bus with the real tiers
// delivered 23 of 25 topics to a `view` token, including pty.desync and pty.exit
// (siblings of the guarded pty.bytes.* stream), agent.statusline for a session
// fleet visibility was hiding, and plugin.log — a verbatim line of sidecar
// stderr, whose environment carries plugin secrets in plaintext.
//
// PLUGINS are no longer exempt for topics the registry names. Their manifest
// `consumes` is a real answer for a topic nobody classified — a plugin-defined
// topic is not host state — but it was NOT an answer for pty.bytes.*: a plugin
// with ZERO capabilities and `consumes: ["pty.bytes.*","fs.changed"]` was
// refused sessions.attachTerminal and fs.watch on the call plane and handed both
// capabilities' whole output here, while the install-consent dialog rendered
// those two lines at severity=normal and hasSensitivePermission() returned
// false. So for a classified topic the manifest is a FILTER, not a grant: a
// guarded topic additionally requires the capability, and a host-only topic is
// refused outright.
func (cn *conn) mayConsume(typ string) bool {
	// Revocation FIRST, before the trusted short-circuit, because an
	// operator-scoped token is promoted to trusted at handshake: a revoked
	// operator socket that kept consuming everything would be the worst case of
	// the very hole this check exists for.
	if cn.revoked.Load() {
		return false
	}
	if cn.trusted {
		return true
	}
	spec, classified := capspec.EventTopicSpec(typ)
	if cn.scopeMethods != nil {
		if !classified {
			// Fail closed. An unclassified topic reaching a scoped user token is
			// the state this whole registry exists to end.
			return false
		}
		switch spec.Disposition {
		case capspec.TopicGuardedBy:
			return event.MatchesAny(cn.scopeMethods, spec.Method)
		case capspec.TopicHostOnly:
			return false
		default:
			return true
		}
	}
	if !event.MatchesAny(cn.consumes, typ) {
		return false
	}
	if !classified {
		return true
	}
	switch spec.Disposition {
	case capspec.TopicGuardedBy:
		_, held := cn.caps[spec.Method]
		return held
	case capspec.TopicHostOnly:
		return false
	default:
		return true
	}
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
	// Revocation first, and before the trusted short-circuit is irrelevant here
	// only because a trusted conn never carries a plugin token: a revoked
	// credential authorizes nothing, whatever it used to authorize.
	if cn.revoked.Load() {
		return false
	}
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
		// REDUNDANT BY CONSTRUCTION, and deliberately kept. RegisterPluginToken
		// `continue`s on capspec.MissingSpec, so such a method never lands in
		// cn.caps, so mayCall denies it before this function is entered — and the
		// two earlier arms (trusted, scoped) return above. There is no path that
		// reaches this line, which is why a mutation deleting it survives the whole
		// tree: that is what a redundant fail-closed check looks like, not a gap
		// (plugin/manager.go expandScope's withinRoot carries the same note). It
		// stays because the invariant it depends on lives in a DIFFERENT function,
		// and the day someone populates caps from anywhere else this is the line
		// that keeps an unspecced filesystem method from running unconfined.
		// TestRegisterRefusesUnspeccedPathCapability pins the invariant itself.
		//
		// The test is MissingSpec, not LooksPathBearing — a method capspec
		// deliberately leaves unconfined (with its reason on the record) is allowed
		// through.
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
		// The ERRNO stays on this side. pathWithinRoots canonicalizes BEFORE it
		// contains, and the walk fails hard on every Lstat error that is not
		// ENOENT — so wrapping that error back to the caller answered a question
		// the grant was supposed to gate, for paths ANYWHERE on the host:
		//
		//   an existing regular file  -> "not a directory"
		//   an unreadable ancestor    -> "permission denied"
		//   absent, or a real dir     -> the containment message below
		//
		// Three distinguishable replies for three out-of-root paths, none of
		// which the plugin was granted, which is a filesystem existence/type/
		// permission oracle for a program confined to one directory. The secret
		// arm two lines up is deliberately non-echoing for exactly this reason;
		// this arm was not, and it is reached FIRST, so the roots never got a
		// say. Same verdict as being outside the scope — which is true: it is
		// outside every root the guard could verify — logged here so the hub
		// operator can still see why a legitimate grant stopped resolving.
		log.Printf("[bus] plugin %q capability %q: %q did not resolve (%v) — denied as out of scope", cn.pluginID, method, target, err)
		return fmt.Errorf("%s: path %q is outside the plugin's granted scope", method, target)
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
