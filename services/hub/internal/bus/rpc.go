package bus

import (
	"encoding/json"
	"strconv"
	"sync"
	"time"
)

// LocalHandler is an in-process capability implementation. Unlike a WebSocket
// provider, it runs inside the hub itself — the seam that lets the hub *own*
// state (e.g. the shared layout document) and answer calls for it directly,
// while still keeping the router generic. The returned value is JSON-encoded
// into the result frame; a non-nil error becomes an error frame.
type LocalHandler func(params json.RawMessage) (any, error)

// callTimeout bounds how long a caller waits for a provider's reply.
const callTimeout = 30 * time.Second

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
	mu        sync.Mutex
	connSeq   uint64
	callSeq   uint64
	conns     map[uint64]*conn
	providers map[string]uint64       // method -> provider conn id
	local     map[string]LocalHandler // method -> in-process handler (hub-owned)
	pending   map[uint64]*pendingCall
	timeout   time.Duration
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
		conns:     make(map[uint64]*conn),
		providers: make(map[string]uint64),
		local:     make(map[string]LocalHandler),
		pending:   make(map[uint64]*pendingCall),
		timeout:   callTimeout,
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
		rt.providers[m] = cn.id
		accepted = append(accepted, m)
	}
	rt.mu.Unlock()
	return accepted
}

// call routes a caller's invocation to the registered provider.
func (rt *router) call(caller *conn, f Frame) {
	if f.Method == "" {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "call missing method"})
		return
	}
	if !caller.mayCall(f.Method) {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "plugin not authorized for capability " + f.Method})
		return
	}
	// Verb is allowed; now enforce argument scoping (e.g. a path-scoped fs.* call
	// must stay within the plugin's granted roots). Fails closed.
	if err := caller.authorize(f.Method, f.Params); err != nil {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: err.Error()})
		return
	}

	// In-process handlers (hub-owned capabilities) take precedence over remote
	// providers. Run off the read loop so a slow handler can't stall the caller's
	// connection, and reply directly with the JSON-encoded result.
	rt.mu.Lock()
	h, isLocal := rt.local[f.Method]
	rt.mu.Unlock()
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
		rt.mu.Unlock()
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "no provider for " + f.Method})
		return
	}
	rt.callSeq++
	gid := rt.callSeq
	p := &pendingCall{caller: caller, corr: f.ID, method: f.Method, providerID: provID}
	p.timer = time.AfterFunc(rt.timeout, func() { rt.timeoutCall(gid) })
	rt.pending[gid] = p
	rt.mu.Unlock()

	// Forward to the provider keyed by the global id.
	if err := provider.send(Frame{
		Op: "call", ID: strconv.FormatUint(gid, 10), Method: f.Method, Params: f.Params,
	}); err != nil {
		rt.failCall(gid, "failed to reach provider for "+f.Method)
	}
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
	_ = caller.send(out)
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
	return len(rt.providers) + len(rt.local)
}

type sendTask struct {
	conn  *conn
	frame Frame
}
