package bus

import (
	"strconv"
	"sync"
	"time"
)

// callTimeout bounds how long a caller waits for a provider's reply.
const callTimeout = 30 * time.Second

// router does request/reply capability routing between connections. Providers
// register method names; callers invoke them; the router forwards the call to
// the owning provider and the reply back to the caller, correlating by a global
// id so different callers can reuse local ids freely.
//
// The hub never executes a capability — it only routes. Authorization is a seam
// (authorize) defaulting to allow-all for now; capability tokens slot in here
// without touching callers or providers.
type router struct {
	mu        sync.Mutex
	connSeq   uint64
	callSeq   uint64
	conns     map[uint64]*conn
	providers map[string]uint64 // method -> provider conn id
	pending   map[uint64]*pendingCall
	timeout   time.Duration

	// authorize reports whether the caller may invoke method. nil = allow all.
	authorize func(callerID uint64, method string) bool
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

func (rt *router) register(cn *conn, methods []string) {
	rt.mu.Lock()
	for _, m := range methods {
		if m != "" {
			rt.providers[m] = cn.id
		}
	}
	rt.mu.Unlock()
}

// call routes a caller's invocation to the registered provider.
func (rt *router) call(caller *conn, f Frame) {
	if f.Method == "" {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "call missing method"})
		return
	}
	if rt.authorize != nil && !rt.authorize(caller.id, f.Method) {
		_ = caller.send(Frame{Op: "error", ID: f.ID, Error: "not authorized for " + f.Method})
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
	return len(rt.providers)
}

type sendTask struct {
	conn  *conn
	frame Frame
}
