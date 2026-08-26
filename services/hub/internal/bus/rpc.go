package bus

import (
	"context"
	"encoding/json"
	"log"
	"slices"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// LocalHandler is an in-process capability implementation. Unlike a WebSocket
// provider, it runs inside the hub itself — the seam that lets the hub *own*
// state (e.g. the shared layout document) and answer calls for it directly,
// while still keeping the router generic. The returned value is JSON-encoded
// into the result frame; a non-nil error becomes an error frame.
type LocalHandler func(params json.RawMessage) (any, error)

// CallerIdentity is who a local handler is answering. The bus is otherwise
// identity-free by design — it routes, it doesn't know what a capability means —
// but a hub-owned capability that stores something OUTLIVING the connection
// needs to record who asked for it. Web Push is the case that forced this: a
// subscription registered by a phone survives every reconnect and every
// restart, so without the subscriber's identity, revoking that phone's token cut
// its bus access while leaving it notified forever.
type CallerIdentity struct {
	// Trusted is the host token (or an operator-tier token, which is promoted to
	// the same authority at the handshake).
	Trusted bool
	// Scope is the tier the connection authenticated as: "operator" for a trusted
	// conn, the tier name for a scoped user token, empty for a plugin.
	Scope string
	// PluginID is set when the caller presented a per-plugin token.
	PluginID string
	// TokenID fingerprints the presented credential (see [TokenFingerprint]).
	// Empty when no token is configured — the loopback default, where there is
	// no credential to identify.
	TokenID string
	// ConnID identifies the calling CONNECTION, not the credential. It exists
	// for the one shape a fingerprint cannot serve: a handler that must
	// describe the other live connections and exclude the one asking, since
	// the caller of "is anything using this machine?" is itself something
	// using this machine. Zero for a call that arrived without a socket.
	ConnID uint64
}

// IsTrusted reports whether the caller holds host authority — the host token, or
// an operator-tier token, which is promoted to the same authority at the
// handshake. Exposed as a METHOD so a package that must ask the question can
// depend on a one-method interface instead of importing the bus (internal/layout
// does exactly that).
func (c CallerIdentity) IsTrusted() bool { return c.Trusted }

// LocalIdentHandler is a [LocalHandler] that also receives the calling
// connection's identity. Register with [Server.RegisterLocalIdent].
type LocalIdentHandler func(caller CallerIdentity, params json.RawMessage) (any, error)

// callTimeout bounds how long a caller waits for a provider's reply.
const callTimeout = 30 * time.Second

// Federation forwards a qualified capability call (`hub:<peer>/<method>`) to a
// peer hub. Implemented by internal/federation; nil = federation off, and every
// qualified call is refused.
type Federation interface {
	// HasPeer reports whether the named peer is configured.
	HasPeer(name string) bool
	// Forward invokes the BARE method on the peer over its federation link and
	// returns the raw result. It must apply its own (shorter-than-callTimeout)
	// budget so the peer-side failure is the one the caller sees, not an
	// ambiguous local timeout.
	Forward(ctx context.Context, peer, method string, params json.RawMessage) (json.RawMessage, error)
}

// splitQualified parses `hub:<peer>/<method>` into its parts. Anything not in
// that exact shape is not a federated call.
func splitQualified(method string) (peer, bare string, ok bool) {
	rest, found := strings.CutPrefix(method, "hub:")
	if !found {
		return "", "", false
	}
	peer, bare, found = strings.Cut(rest, "/")
	if !found || peer == "" || bare == "" {
		return "", "", false
	}
	return peer, bare, true
}

// router does request/reply capability routing between connections. Providers
// register method names; callers invoke them; the router forwards the call to
// the owning provider and the reply back to the caller, correlating by a global
// id so different callers can reuse local ids freely.
//
// The hub never executes a capability — it only routes. Authorization is per
// connection: the bus tags each conn at handshake as trusted (host token) or as
// a specific plugin (per-plugin token) with a fixed set of allowed capabilities;
// call() consults that set via conn.mayCall.
type router struct {
	mu         sync.Mutex
	connSeq    uint64
	callSeq    uint64
	conns      map[uint64]*conn
	providers  map[string]uint64            // method -> provider conn id
	local      map[string]LocalHandler      // method -> in-process handler (hub-owned)
	localIdent map[string]LocalIdentHandler // same, for handlers that need the caller
	pending    map[uint64]*pendingCall
	timeout    time.Duration
	// fed forwards qualified `hub:<peer>/<method>` calls; nil = federation off.
	fed Federation
	// noProviderSeen remembers which methods have already been reported as
	// unprovided, so the log line fires once per method rather than once per
	// call. Cleared for a method the moment something registers it.
	noProviderSeen map[string]struct{}
}

type pendingCall struct {
	caller     *conn
	corr       string // caller's original id
	method     string
	providerID uint64
	timer      *time.Timer
}

func newRouter() *router {
	return &router{
		conns:          make(map[uint64]*conn),
		providers:      make(map[string]uint64),
		local:          make(map[string]LocalHandler),
		localIdent:     make(map[string]LocalIdentHandler),
		pending:        make(map[uint64]*pendingCall),
		timeout:        callTimeout,
		noProviderSeen: make(map[string]struct{}),
	}
}

func (rt *router) addConn(cn *conn) {
	rt.mu.Lock()
	rt.connSeq++
	cn.id = rt.connSeq
	rt.conns[cn.id] = cn
	rt.mu.Unlock()
}

// dropConn removes a connection: unregister any methods it provided and fail
// any calls that depended on it (caller gone → drop; provider gone → error to
// caller).
func (rt *router) dropConn(cn *conn) {
	rt.mu.Lock()
	delete(rt.conns, cn.id)
	for m, id := range rt.providers {
		if id == cn.id {
			delete(rt.providers, m)
		}
	}
	var notify []sendTask
	for gid, p := range rt.pending {
		switch cn.id {
		case p.caller.id:
			p.timer.Stop()
			delete(rt.pending, gid)
		case p.providerID:
			p.timer.Stop()
			delete(rt.pending, gid)
			notify = append(notify, sendTask{p.caller, Frame{
				Op: "error", ID: p.corr, Error: "provider for " + p.method + " disconnected",
			}})
		}
	}
	rt.mu.Unlock()
	for _, t := range notify {
		_ = t.conn.send(t.frame)
	}
}

// registerLocal installs an in-process capability handler. Local handlers take
// precedence over WebSocket providers for the same method name.
func (rt *router) registerLocal(method string, h LocalHandler) {
	rt.mu.Lock()
	rt.local[method] = h
	delete(rt.localIdent, method)
	rt.mu.Unlock()
}

// registerLocalIdent installs an in-process handler that is told who called it.
func (rt *router) registerLocalIdent(method string, h LocalIdentHandler) {
	rt.mu.Lock()
	rt.localIdent[method] = h
	delete(rt.local, method)
	rt.mu.Unlock()
}

// register installs cn as the provider for each method it's allowed to provide
// (trusted conns: all; plugins: those matched by their `provides` grant).
// Returns the methods actually registered, so the caller's ack is truthful and a
// plugin can tell which of its requested methods were withheld.
func (rt *router) register(cn *conn, methods []string) []string {
	accepted := make([]string, 0, len(methods))
	rt.mu.Lock()
	for _, m := range methods {
		if m == "" || !cn.mayProvide(m) {
			continue
		}
		// First-registration-wins (capability-hijack guard): a method already owned
		// by a *different, still-live* connection may not be re-registered. Without
		// this, any token-bearing client — a plugin, or a remote client under the
		// shared host token — could re-register claude.approve / agents.spawn and
		// silently intercept every subsequent caller's params (session ids, prompts,
		// approvals). The check applies to trusted conns too, because a remote client
		// presenting the host token is itself `trusted`, so exempting trusted callers
		// would reopen the exact hole. Ownership is released the instant the owner's
		// connection drops: dropConn deletes its providers under this same mutex, so
		// the desktop's own reconnect (and any provider restart) re-registers cleanly
		// into the now-empty slot — only a live owner is protected. Re-registering a
		// method you already own is idempotent (ownerID == cn.id falls through).
		if ownerID, owned := rt.providers[m]; owned && ownerID != cn.id {
			if _, live := rt.conns[ownerID]; live {
				continue // owned by another live connection — refuse the hijack
			}
		}
		// A hub-LOCAL handler shadows every remote provider (call() consults
		// rt.local/rt.localIdent first), so accepting one here would assert an
		// ownership the router will never honour. That matters beyond the hub:
		// the desktop derives its ENTIRE drift report — the only place it can
		// notice it lost a capability — from this ack (hubClient.ts), so a
		// method the hub quietly took in-process would leave the desktop
		// believing it provides something it does not.
		if _, local := rt.local[m]; local {
			continue
		}
		if _, local := rt.localIdent[m]; local {
			continue
		}
		rt.providers[m] = cn.id
		delete(rt.noProviderSeen, m) // say it again if this provider goes away
		accepted = append(accepted, m)
	}
	rt.mu.Unlock()
	return accepted
}

// spawnMethod is the one capability whose params the router rewrites. The bus
// is otherwise deliberately payload-blind — it routes, it doesn't interpret —
// but profile dispatch is a per-TOKEN grant, and the token is only verifiable
// here, so the router is the single place the grant can be turned into
// something a provider may trust without re-verifying credentials it never
// sees.
const spawnMethod = "agents.spawn"

// reportProgressMethod is the second, for the same structural reason and the
// mirror-image rewrite. `callerSessionId` is not a target — it is the caller
// ASSERTING WHO IT IS, and the provider turns that assertion into a recipient
// (the named session's own parent). An assertion of identity is exactly the
// thing a caller may not make about itself, so the router deletes it from every
// caller that is not the control plane. The MCP facade, which IS the control
// plane, stamps it from the per-request token record's `session:<id>` label
// before the call ever reaches this connection — the same "the facade resolved
// the session, the hub cannot" split spawn's yolo/profile grants have.
const reportProgressMethod = "agents.reportProgress"

// mayUseProfile reports whether this connection may dispatch an agent under
// the named Claude profile.
//
//   - host token (trusted, NOT via tokens.json): yes, any profile. This is the
//     control plane's own credential — the desktop, the MCP facade (which
//     enforces per-session facade-token grants itself before a profileId ever
//     reaches its bus connection), the brain. A process holding it could
//     rewrite tokens.json, so gating it here would be theater.
//   - scoped user token (tokens.json), operator tier included: only ids in the
//     record's profilesAllowed grant. Operator promotion to `trusted` grants
//     METHODS, not profiles — the fleet-manager grant is per-token by design,
//     so two operator sessions can hold different account sets.
//   - plugin token: never. A plugin's consent dialog never mentioned accounts.
func (cn *conn) mayUseProfile(id string) bool {
	if id == "" || cn.revoked.Load() {
		return false
	}
	if cn.pluginID != "" {
		return false
	}
	if cn.federated {
		// Same reasoning as mayBypassPermissions: a forwarded spawn arrives on
		// the link's connection, and peers.json routinely holds the far hub's
		// HOST token. Naming an ACCOUNT to burn is a grant the far hub must have
		// recorded on the link's own record, not one the link inherits from
		// being authenticated.
		return cn.viaScopedToken && slices.Contains(cn.profilesAllowed, id)
	}
	if cn.viaScopedToken {
		// DELIBERATELY NOT WIDENED to "operator tier may name any profile", even
		// though mayBypassPermissions now is (2026-08-26). The two look alike
		// and are not: yoloAllowed is a PERMISSION LEVEL, profilesAllowed is an
		// ACCOUNT ALLOWLIST, and operator-tier tokens are exactly what the
		// account allowlist is enforced against today — every per-session facade
		// token the desktop mints (claudeSpawn.ts mintSessionFacadeToken) is
		// operator-scoped, and only a `manager` gets a profile list at all. So
		// "operator ⇒ any profile" would erase the fleet-manager grant wholesale
		// and let any facade worker spawn as any of the user's Claude accounts.
		// A profile is which identity/billing account runs the work; a bypass is
		// how much the work may do without asking. The host token and an
		// explicitly-granted record keep naming profiles; everyone else is told
		// it was dropped (see sanitizeSpawnParams' escalationScrubbed stamp)
		// rather than silently spawned under the default account.
		return slices.Contains(cn.profilesAllowed, id)
	}
	return cn.trusted
}

// PeerLinkParam is the query param a FEDERATION LINK sets on its bus handshake
// (`?peer=1`) to declare itself one. internal/federation sets it; every other
// client (desktop, brain, MCP facade, plugin sidecar, phone) does not.
//
// Self-asserted on purpose. It is read ONLY to WITHHOLD authority — see
// [conn.mayBypassPermissions] — so the worst a client can do by lying is give
// itself less than its credential carries, which needs no proof. The honest
// half (a link that does NOT set it) is covered by the far hub minting the link
// a token whose grants it actually means, which was always the ceiling.
const PeerLinkParam = "peer"

// mayBypassPermissions reports whether this connection holds the full-access
// grant: whether an agents.spawn it sends may have its skipPermissions request
// honored by the provider.
//
// THE TOKEN IS THE TRUST BOUNDARY. The product rule this encodes (2026-08-26)
// is that a remote client should feel like sitting at the machine, so the
// credential decides — not the fact of being remote:
//
//   - plugin token: never. A plugin's consent dialog never offered this, and a
//     plugin is third-party code, not the user.
//   - FEDERATION LINK (`?peer=1`): only when the link's own token record
//     carries an explicit yoloAllowed grant. A peer's forwarded spawn re-enters
//     this router on the link connection, and peers.json routinely holds the
//     far hub's HOST token — so without this clause "the link is authenticated"
//     would silently mean "every spawn any peer forwards runs bypassed",
//     inheriting host trust nobody granted per-call. The far hub must mint the
//     link a token that SAYS full access before it means it.
//   - operator-tier token (viaScopedToken && trusted): YES. ScopeOperator is
//     documented as "everything — equivalent to the host remote-token", so
//     clamping it was the silent downgrade the user hit: a full-access spawn
//     from the phone came up in ask-mode with only a server log to say so.
//   - any other scoped token: only with yoloAllowed. (view/triage cannot reach
//     agents.spawn at all; the clause is the belt for a hand-edited record.)
//   - host token: yes — the control plane's own credential.
//
// PROFILE dispatch (mayUseProfile) deliberately does NOT follow this widening;
// see the note there for why an account allowlist is the one grant an operator
// token must still hold per-token.
func (cn *conn) mayBypassPermissions() bool {
	if cn.revoked.Load() {
		return false
	}
	if cn.pluginID != "" {
		return false
	}
	if cn.federated {
		return cn.viaScopedToken && cn.yoloAllowed
	}
	if cn.viaScopedToken {
		// trusted && viaScopedToken is exactly the operator tier: the handshake
		// promotes ScopeOperator to trusted and nothing else in the scoped
		// branch sets it (bus.go handleBus, si.operator()).
		return cn.yoloAllowed || cn.trusted
	}
	return cn.trusted
}

// sanitizeSpawnParams enforces the profile-dispatch grant on an agents.spawn's
// params, at the router's single dispatch point:
//
//  1. `profileGranted` is DELETED from every incoming call. It is hub-stamped
//     only — a provider seeing it true knows the hub verified the caller, and
//     no caller (spoofing included) can be its source.
//  2. `profileId` survives only when the caller may use that exact profile
//     (mayUseProfile); the hub then stamps `profileGranted: true` beside it.
//     Otherwise the field is stripped, which is byte-for-byte today's doctrine:
//     an ungranted bus caller cannot name a profile at all.
//  3. `yoloGranted` is likewise DELETED from every incoming call and hub-
//     stamped true only for a caller holding the full-access grant
//     (mayBypassPermissions). The stamp is about the CALLER, not the request:
//     `skipPermissions` itself passes through untouched either way — callers
//     keep requesting it, the stamp says the provider may honor it, and an
//     unstamped request keeps today's clamp.
//  4. `escalationScrubbed` is DELETED from every incoming call (hub-stamped
//     only, same as the two above) and re-stamped with what THIS router took
//     away — today just `profileId`. NO SILENT DOWNGRADES: the provider folds
//     the stamp together with its own clamps and returns the union as the spawn
//     result's `escalationScrubbed`, so a caller that asked for full access and
//     did not get it learns so from the ANSWER instead of from a log line on a
//     machine it cannot read. An empty/absent stamp means "the hub took
//     nothing".
//
// Non-object params pass through untouched — there is no field to smuggle in a
// shape the provider's own decoder would reject anyway. The provider keeps its
// own scrubs (configDir never comes from the wire; the profile id resolves
// against the provider's LOCAL profile store), so this is an additional gate in
// front of them, not a replacement.
func sanitizeSpawnParams(caller *conn, raw json.RawMessage) json.RawMessage {
	if len(raw) == 0 {
		return raw
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(raw, &m); err != nil || m == nil {
		return raw
	}
	delete(m, "profileGranted")
	delete(m, "yoloGranted")
	delete(m, "escalationScrubbed")
	if caller.mayBypassPermissions() {
		m["yoloGranted"] = json.RawMessage("true")
	} else if caller.federated {
		// Said out loud, and with the remedy in it: this is the ONE denial a
		// correctly-credentialled operator can hit without having done anything
		// wrong, because the ceiling is the LINK's token rather than theirs.
		log.Printf("SECURITY: agents.spawn: full access withheld from federation link %s — a peer link inherits no host trust; mint its token with `workspacer token create --scope operator --full-access` on this machine and put THAT token in the peer's peers.json entry", caller.tokenID)
	}
	var scrubbed []string
	var pid string
	hadProfile := false
	if r, ok := m["profileId"]; ok {
		hadProfile = true
		if json.Unmarshal(r, &pid) != nil {
			pid = "" // non-string spelling: strip rather than interpret
		}
	}
	if pid != "" && caller.mayUseProfile(pid) {
		m["profileGranted"] = json.RawMessage("true")
	} else {
		delete(m, "profileId")
		if hadProfile {
			scrubbed = append(scrubbed, "profileId")
		}
	}
	if len(scrubbed) > 0 {
		if raw, err := json.Marshal(scrubbed); err == nil {
			m["escalationScrubbed"] = raw
		}
	}
	out, err := json.Marshal(m)
	if err != nil {
		// Cannot happen for a map of valid RawMessages; fail closed anyway by
		// refusing to forward the un-sanitized original's profile fields.
		return json.RawMessage("{}")
	}
	return out
}

// sanitizeReportProgressParams deletes `callerSessionId` from an untrusted
// caller's agents.reportProgress params. There is no stamping half: the bus
// connection of a scoped or plugin token carries no session identity to stamp
// from, so the honest result is ABSENCE — and absence is not silent, because the
// provider refuses a report it cannot attribute with a message saying exactly
// that ("the host could not identify your session from your credential"). What
// this closes is a plugin or phone token forging a progress wake FROM any worker
// TO its manager, which is the one thing a one-way channel with a host-derived
// recipient would otherwise still allow.
//
// Trusted callers pass through: the desktop, the brain and the MCP facade are
// the control plane, and the facade in particular multiplexes every session over
// one host-token connection, so it is the only party that CAN name the session
// (from the token record it resolved). Non-object params pass through untouched.
func sanitizeReportProgressParams(caller *conn, raw json.RawMessage) json.RawMessage {
	if caller.trusted || len(raw) == 0 {
		return raw
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(raw, &m); err != nil || m == nil {
		return raw
	}
	if _, ok := m["callerSessionId"]; !ok {
		return raw
	}
	delete(m, "callerSessionId")
	out, err := json.Marshal(m)
	if err != nil {
		// Cannot happen for a map of valid RawMessages; fail closed anyway.
		return json.RawMessage("{}")
	}
	return out
}

// paramSanitizer rewrites a call's params based on the VERIFIED caller. Keyed
// by the BARE method name in [methodSanitizers] below.
type paramSanitizer func(caller *conn, raw json.RawMessage) json.RawMessage

// methodSanitizers is the single source of truth for which capabilities have
// caller-identity fields the router must strip or stamp rather than forward
// verbatim. Both call() and federatedCall() dispatch through
// [sanitizeCallParams] against this ONE map — neither path hand-lists methods
// itself — so a sanitizer added here automatically covers the federated hop
// too, and the two paths cannot drift apart by construction. Before this, each
// dispatch point repeated its own `if method == X` list by hand, and the
// federated one silently forgot the field-stripping half of
// agents.reportProgress the day it was added to the view tier: see
// TestReportProgressCallerSessionIsStrippedBeforeTheFederatedHop.
var methodSanitizers = map[string]paramSanitizer{
	spawnMethod:          sanitizeSpawnParams,
	reportProgressMethod: sanitizeReportProgressParams,
}

// sanitizeCallParams applies method's sanitizer, if any, against the VERIFIED
// caller. Methods with no entry in [methodSanitizers] pass through untouched.
func sanitizeCallParams(caller *conn, method string, raw json.RawMessage) json.RawMessage {
	if s, ok := methodSanitizers[method]; ok {
		return s(caller, raw)
	}
	return raw
}

// call routes a caller's invocation to the registered provider.
func (rt *router) call(caller *conn, f Frame) {
	if f.Method == "" {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "call missing method"})
		return
	}
	// Federated calls (`hub:<peer>/<method>`) take their own authorization path
	// and never touch the local provider table.
	if peer, bare, ok := splitQualified(f.Method); ok {
		rt.federatedCall(caller, f, peer, bare)
		return
	}
	if !caller.mayCall(f.Method) {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: caller.callDenied(f.Method)})
		return
	}
	// Verb is allowed; now enforce argument scoping (e.g. a path-scoped fs.* call
	// must stay within the plugin's granted roots). Fails closed.
	if err := caller.authorize(f.Method, f.Params); err != nil {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: err.Error()})
		return
	}
	// Identity-assertion / grant fields the caller may not set for itself
	// (profile dispatch, the reportProgress session stamp, ...): strip/stamp
	// them based on the VERIFIED caller, before either dispatch path (local or
	// provider) can see them. See [methodSanitizers].
	f.Params = sanitizeCallParams(caller, f.Method, f.Params)

	// In-process handlers (hub-owned capabilities) take precedence over remote
	// providers. Run off the read loop so a slow handler can't stall the caller's
	// connection, and reply directly with the JSON-encoded result.
	rt.mu.Lock()
	h, isLocal := rt.local[f.Method]
	hi, isLocalIdent := rt.localIdent[f.Method]
	rt.mu.Unlock()
	if isLocalIdent {
		h, isLocal = func(params json.RawMessage) (any, error) {
			return hi(caller.identity(), params)
		}, true
	}
	if isLocal {
		go func() {
			res, err := h(f.Params)
			if err != nil {
				_ = caller.send(Frame{Op: "error", ID: f.ID, Error: err.Error()})
				return
			}
			raw, mErr := json.Marshal(res)
			if mErr != nil {
				_ = caller.send(Frame{Op: "error", ID: f.ID, Error: mErr.Error()})
				return
			}
			_ = caller.send(Frame{Op: "result", ID: f.ID, Result: raw})
		}()
		return
	}

	rt.mu.Lock()
	provID, ok := rt.providers[f.Method]
	provider := rt.conns[provID]
	if !ok || provider == nil {
		first := false
		if _, seen := rt.noProviderSeen[f.Method]; !seen {
			rt.noProviderSeen[f.Method] = struct{}{}
			first = true
		}
		rt.mu.Unlock()
		if first {
			// ONCE per method, so a client in a retry loop cannot drown the log
			// while a genuinely missing plane still gets said out loud. Until
			// this existed, an entire capability plane could die and the only
			// trace anywhere was a per-call error string the calling code
			// usually swallows: no hub log line, no bus event, nothing.
			log.Printf("bus: NO PROVIDER for %q — nothing on this bus answers it; the caller sees an error string and nobody else sees anything (this is logged once per method)", f.Method)
		}
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "no provider for " + f.Method})
		return
	}
	rt.callSeq++
	gid := rt.callSeq
	p := &pendingCall{caller: caller, corr: f.ID, method: f.Method, providerID: provID}
	p.timer = time.AfterFunc(rt.timeout, func() { rt.timeoutCall(gid) })
	rt.pending[gid] = p
	rt.mu.Unlock()

	// Forward to the provider keyed by the global id, through this caller's
	// ordered dispatch queue: off the read loop, so an unresponsive provider can't
	// head-of-line block everything else this client sends, but still in the order
	// this client sent them — a goroutine per forward would let one caller's
	// successive calls arrive inverted, and calls on this path (terminal input) are
	// order-sensitive. See [conn.forward]. The pending entry is registered above,
	// so a reply can't outrun the forward.
	queued := caller.forward(forwardTask{
		target: provider,
		frame:  Frame{Op: "call", ID: strconv.FormatUint(gid, 10), Method: f.Method, Params: f.Params},
		onErr:  func() { rt.failCall(gid, "failed to reach provider for "+f.Method) },
	})
	if !queued {
		rt.failCall(gid, "too many calls queued behind an unresponsive provider for "+f.Method)
	}
}

// federatedCall forwards `hub:<peer>/<bare>` over the peer's federation link.
//
// Authorization, deliberately different per credential kind:
//
//   - trusted (host token / operator tier): allowed — same authority the bare
//     method would get locally.
//   - scoped user token: the tier check runs against the BARE method, so a
//     view token may call hub:work/agents.list exactly when it may call
//     agents.list. The tier allowlists stay exact-name (never globs), and the
//     peer-side link token is a second, independent ceiling.
//   - plugin token: refused outright. A plugin's consented grant names what it
//     may reach ON THIS MACHINE; silently extending `agents.list` to every
//     configured peer would widen a consent the user never gave. If plugin
//     federation is ever wanted, it must be a distinct, explicitly-consented
//     grant shape — not prefix-stripping leniency.
//
// Local argument confinement (authorize) is deliberately NOT applied: paths in
// a federated call name the PEER's filesystem, and canonicalizing them against
// the local one would both reject valid calls and approve invalid ones. The
// peer enforces its own confinement against the link token's grants.
func (rt *router) federatedCall(caller *conn, f Frame, peer, bare string) {
	rt.mu.Lock()
	fed := rt.fed
	rt.mu.Unlock()
	if fed == nil {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "federation not configured"})
		return
	}
	if !fed.HasPeer(peer) {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "unknown federation peer " + strconv.Quote(peer)})
		return
	}
	id := caller.identity()
	switch {
	case id.PluginID != "":
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "plugin " + id.PluginID + " may not call federated capabilities"})
		return
	case id.Trusted:
		// host authority — allowed
	default:
		if !caller.mayCall(bare) {
			_ = caller.send(Frame{Op: "error", ID: f.ID, Error: caller.callDenied(bare)})
			return
		}
	}
	// Same sanitize table as the local dispatch point, applied against the
	// BARE method — and applied AGAIN by the peer against its federation-link
	// connection when the forwarded call re-enters the peer's own router. Both
	// layers are load-bearing: this one keeps an ungranted local caller from
	// riding the link's authority; the peer's keeps a granted-here id/stamp from
	// meaning anything there unless the link credential is trusted with it.
	// (Path confinement is deliberately NOT applied to federated params — see
	// the comment above — but these fields are grants and identity assertions
	// this router CAN verify, not paths naming the peer's filesystem.)
	//
	// This MUST go through the same [methodSanitizers] table as call() rather
	// than its own hand-written list: the local tier check above runs against
	// the bare method, so any tier that may call a sanitized method locally may
	// call hub:<peer>/<method> too — and on the far side that arrives on the
	// peer's federation-link connection, which is trusted whenever the peer
	// minted an operator-tier link token, so the peer's own sanitizer exempts
	// it. A hand-listed pair here that forgot one entry is exactly how
	// agents.reportProgress's callerSessionId briefly rode across ungated: see
	// TestReportProgressCallerSessionIsStrippedBeforeTheFederatedHop.
	f.Params = sanitizeCallParams(caller, bare, f.Params)
	// Off the read loop: the forward blocks on the peer's reply. The forwarder
	// owns the (shorter) timeout, so the failure the caller sees names the
	// federated hop rather than an ambiguous local deadline.
	go func() {
		res, err := fed.Forward(context.Background(), peer, bare, f.Params)
		if err != nil {
			_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "hub:" + peer + ": " + err.Error()})
			return
		}
		_ = caller.send(Frame{Op: "result", ID: f.ID, Result: res})
	}()
}

// result routes a provider's reply (result or error) back to the caller.
func (rt *router) result(provider *conn, f Frame, isError bool) {
	gid, err := strconv.ParseUint(f.ID, 10, 64)
	if err != nil {
		return // not a hub-assigned id; ignore
	}
	rt.mu.Lock()
	p, ok := rt.pending[gid]
	if !ok || p.providerID != provider.id {
		rt.mu.Unlock()
		return // unknown call, or a different conn impersonating the provider
	}
	p.timer.Stop()
	delete(rt.pending, gid)
	caller := p.caller
	corr := p.corr
	rt.mu.Unlock()

	out := Frame{ID: corr}
	if isError {
		out.Op = "error"
		out.Error = f.Error
	} else {
		out.Op = "result"
		out.Result = f.Result
	}
	// Off the provider's read loop: the reply crosses to another connection, so
	// sending it inline lets one slow caller stall every other call this provider
	// is answering. Deliberately a bare goroutine rather than the provider's
	// ordered forward queue, which is the opposite choice from the call path
	// above: replies from one provider go to DIFFERENT callers, so ordering them
	// would put a wedged caller's reply in front of everyone else's — precisely
	// the head-of-line stall this is here to avoid. Nothing is lost by it, since
	// callers correlate replies by id and events already arrive on their own
	// goroutine, so there was never a total order to preserve.
	go func() { _ = caller.send(out) }()
}

func (rt *router) timeoutCall(gid uint64) {
	rt.failCall(gid, "call timed out")
}

func (rt *router) failCall(gid uint64, msg string) {
	rt.mu.Lock()
	p, ok := rt.pending[gid]
	if !ok {
		rt.mu.Unlock()
		return
	}
	p.timer.Stop()
	delete(rt.pending, gid)
	caller := p.caller
	corr := p.corr
	rt.mu.Unlock()
	_ = caller.send(Frame{Op: "error", ID: corr, Error: msg})
}

func (rt *router) methodCount() int {
	rt.mu.Lock()
	defer rt.mu.Unlock()
	return len(rt.providers) + len(rt.local) + len(rt.localIdent)
}

// providedMethods is the sorted union of every method this bus can currently
// answer: remote providers plus the hub's own in-process handlers. A bare COUNT
// (what /health used to expose alone) tells nobody whether the plane they need
// is alive; a NAME LIST lets `workspacer status`, a client, or an operator ask
// the one question that matters — "is what I am about to call actually
// provided?" — before the call fails as a string.
func (rt *router) providedMethods() []string {
	rt.mu.Lock()
	out := make([]string, 0, len(rt.providers)+len(rt.local)+len(rt.localIdent))
	for m := range rt.providers {
		out = append(out, m)
	}
	for m := range rt.local {
		out = append(out, m)
	}
	for m := range rt.localIdent {
		out = append(out, m)
	}
	rt.mu.Unlock()
	sort.Strings(out)
	return out
}

type sendTask struct {
	conn  *conn
	frame Frame
}

// setFederation wires the federation forwarder into the router.
func (rt *router) setFederation(f Federation) {
	rt.mu.Lock()
	rt.fed = f
	rt.mu.Unlock()
}
