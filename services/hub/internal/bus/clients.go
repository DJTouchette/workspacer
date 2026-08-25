package bus

// Live-connection reporting: who is CONNECTED to this hub right now and when
// each of them last did something.
//
// This exists for one caller — the hub's fleet-quiescence signal — and the
// distinction it draws is the whole reason it is not just `len(conns)`.
// "Somebody is using this machine" and "a socket is open" are different
// questions, and answering the first with the second would be wrong in both
// directions: a phone that has held a socket open in a background tab since
// yesterday is not use, and the hub's own loopback client and every capability
// provider are not use either, though both are permanently connected.

import (
	"net/http"
	"sort"
	"time"
)

// ClientInfo describes one live connection in the words an operator would use.
// It carries no credential: `Label` is a description, never a token, and the
// per-connection fingerprint stays where it already lives.
type ClientInfo struct {
	// ConnID identifies the connection, so a caller can exclude its own.
	ConnID uint64
	// Label describes what is on the other end.
	Label string
	// LastActive is when this connection last called a capability or published
	// an event, or when it connected, whichever is later.
	LastActive time.Time
	// ActivitySeq counts the same events as LastActive, one per act, as a
	// strictly increasing integer. A caller that needs to tell "this exact act"
	// from "anything since" needs this rather than LastActive: two acts on a
	// loopback connection routinely share a millisecond, and the wall clock
	// cannot tell them apart where this counter always can.
	ActivitySeq uint64
	// Provider is true when this connection answers capability calls for
	// somebody — the Electron main process, the headless brain, a plugin
	// sidecar that registered a method. A provider is infrastructure: it is
	// connected because the machine is running, not because anyone is using it.
	Provider bool
	// Plugin is true for a per-plugin token. Also infrastructure: a sidecar is
	// started and stopped by the hub, not by a person.
	Plugin bool
	// Internal is true for the hub's own loopback client.
	Internal bool
}

// UserFacing reports whether this connection represents somebody USING the
// machine, as opposed to a piece of the machine being connected to itself.
func (c ClientInfo) UserFacing() bool {
	return !c.Provider && !c.Plugin && !c.Internal
}

// Clients snapshots every live connection. Order is by connection id, so two
// readings of an unchanged hub render identically.
func (s *Server) Clients() []ClientInfo {
	providers := s.router.providerConns()
	s.router.mu.Lock()
	out := make([]ClientInfo, 0, len(s.router.conns))
	for id, cn := range s.router.conns {
		out = append(out, ClientInfo{
			ConnID:      id,
			Label:       cn.describe(),
			LastActive:  time.UnixMilli(cn.lastActiveMilli.Load()),
			ActivitySeq: cn.activitySeq.Load(),
			Provider:    providers[id],
			Plugin:      cn.pluginID != "",
			Internal:    cn.internal,
		})
	}
	s.router.mu.Unlock()
	sort.Slice(out, func(i, j int) bool { return out[i].ConnID < out[j].ConnID })
	return out
}

// ConnActivitySeq reads one connection's current [ClientInfo.ActivitySeq]. For a
// local handler that just recorded a caller's ConnID and needs to know exactly
// which act that was, so it can tell whether the connection has done anything
// ELSE the next time it is asked — see fleet.quiescence's askedSeq for why the
// wall clock cannot serve that comparison.
func (s *Server) ConnActivitySeq(connID uint64) (uint64, bool) {
	rt := s.router
	rt.mu.Lock()
	defer rt.mu.Unlock()
	cn, ok := rt.conns[connID]
	if !ok {
		return 0, false
	}
	return cn.activitySeq.Load(), true
}

// describe names the other end of a connection without naming its credential.
func (cn *conn) describe() string {
	switch {
	case cn.pluginID != "":
		return "plugin " + cn.pluginID
	case cn.internal:
		return "the hub's own loopback client"
	case cn.scope != "":
		return cn.scope + "-tier client"
	case cn.trusted:
		return "operator client"
	default:
		return "client"
	}
}

// providerConns returns the ids of connections that answer capability calls.
func (rt *router) providerConns() map[uint64]bool {
	out := map[uint64]bool{}
	rt.mu.Lock()
	for _, id := range rt.providers {
		out[id] = true
	}
	rt.mu.Unlock()
	return out
}

// internalDialParam is the query parameter the hub's own loopback client
// presents so its connection can be told apart from a user's.
const internalDialParam = "internal"

// SetInternalKey installs the nonce that marks the hub's own loopback bus
// client. Generate it per process (never persist it, never log it) and append
// it to the self-dial URL as ?internal=<key>.
//
// This is provenance, not authorization: the self-client already holds the
// host token, so the key grants nothing. What it buys is that the hub's own
// machinery — the jobs runner, the quiescence sampler — does not read as
// somebody using the machine. Without it, a hub asking itself whether anything
// is happening would find its own question and answer yes, forever.
func (s *Server) SetInternalKey(key string) {
	s.intMu.Lock()
	s.internalKey = key
	s.intMu.Unlock()
}

func (s *Server) isInternalDial(r *http.Request) bool {
	s.intMu.RLock()
	key := s.internalKey
	s.intMu.RUnlock()
	if key == "" {
		return false
	}
	return r.URL.Query().Get(internalDialParam) == key
}

// InternalDialURL appends the internal marker to a self-dial bus URL.
func InternalDialURL(busURL, key string) string {
	if key == "" {
		return busURL
	}
	sep := "?"
	for i := 0; i < len(busURL); i++ {
		if busURL[i] == '?' {
			sep = "&"
			break
		}
	}
	return busURL + sep + internalDialParam + "=" + key
}

// ProviderConnID returns the id of the connection currently registered as the
// provider for method. Its one caller is the node supervisor's liveness poll:
// to decide that a provider has gone away without saying so, you first have to
// be able to name the connection you are accusing.
func (s *Server) ProviderConnID(method string) (uint64, bool) {
	rt := s.router
	rt.mu.Lock()
	defer rt.mu.Unlock()
	id, ok := rt.providers[method]
	if !ok {
		return 0, false
	}
	if _, live := rt.conns[id]; !live {
		return 0, false
	}
	return id, true
}

// EvictConn force-closes a connection and releases everything it owned,
// returning whether there was anything to evict.
//
// THIS IS THE ZOMBIE-PROVIDER RELEASE, and it exists because the ordinary one
// cannot be relied on. Provider ownership is first-registration-wins and is
// released by dropConn, which runs when the hub's READ LOOP returns — i.e.
// when the socket is observed to close. A provider that never subscribes to a
// topic is never written to unprompted, so a failed write can never reveal a
// dead far end either. Put those together and a machine that stops without its
// TCP connection being severed cleanly leaves the hub holding a registration
// slot forever, and REFUSES the same machine's re-registration when it comes
// back up: the node boots and provides nothing.
//
// So the hub is given a way to decide for itself. The caller establishes the
// far end is gone (the node supervisor calls brain.info and gets no answer
// inside a deadline) and evicts.
//
// Ownership is released SYNCHRONOUSLY, before this returns, rather than being
// left to the read loop's deferred dropConn: a wake path evicts and then
// starts a machine, and the machine's brain may dial in within a second. The
// deferred dropConn still runs when the read loop unwinds; it finds nothing
// left to do, which is harmless.
//
// It grants nothing and is not reachable over the bus: no capability method is
// wired to it. It is an in-process decision by the hub about its own sockets.
func (s *Server) EvictConn(connID uint64) bool {
	rt := s.router
	rt.mu.Lock()
	cn, ok := rt.conns[connID]
	rt.mu.Unlock()
	if !ok {
		return false
	}
	// Close first so a far end that IS alive cannot re-register into the slot
	// between the release below and the socket going away.
	_ = cn.ws.CloseNow()
	rt.dropConn(cn)
	return true
}
