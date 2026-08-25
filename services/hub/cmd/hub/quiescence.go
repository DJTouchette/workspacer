package main

// The hub-side wiring for the fleet-quiescence signal: gather a reading of the
// machine every tick, fold it into the dwell, and answer `fleet.quiescence`
// from the latest one.
//
// It lives in the hub rather than in the brain or the desktop because three of
// the four things the predicate reads are the hub's own — the live bus
// connections, the job schedule, and the federation links — and because the
// hub is the one process that exists in every deployment. The fourth, the
// session rows, comes over the bus from whichever provider is live, so the
// same code answers whether the desktop or the headless brain is serving them.
//
// Everything here is READ-ONLY. Nothing in this file, or reachable from it,
// stops, suspends or otherwise touches the machine: the signal is published and
// the decision belongs entirely to whatever the operator wires up to it.

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/federation"
	"github.com/djtouchette/workspacer-hub/internal/jobs"
	"github.com/djtouchette/workspacer-hub/internal/quiescence"
)

// fleetProbeTimeout bounds one reading. A provider that does not answer inside
// it makes the fleet unreadable, which BLOCKS — a slow provider must never
// resolve to "nothing is happening".
const fleetProbeTimeout = 10 * time.Second

// sampleIdleAfter is how long the sampler keeps taking readings after the last
// time anybody asked for one.
//
// The sampler exists to make "held continuously" mean something, and it pays
// for that with a `sessions.snapshots` call every tick. Nobody should pay it
// who is not using the signal: on an ordinary desktop install nothing ever
// calls fleet.quiescence, and a permanent background poll for an answer no one
// reads is exactly the kind of ambient cost this feature is supposed to
// REMOVE. So the first ask starts the sampler and it winds down again once the
// asking stops. Comfortably longer than any sensible polling interval, so a
// poller never finds it dormant twice in a row.
const sampleIdleAfter = 15 * time.Minute

// newInternalKey mints the per-process nonce that marks the hub's own loopback
// bus client. Never persisted, never logged: it grants nothing (the self-client
// carries the host token regardless), it only lets the hub recognise itself.
func newInternalKey() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return ""
	}
	return hex.EncodeToString(b)
}

// fleetWatcher samples the machine and holds the dwell.
type fleetWatcher struct {
	srv  *bus.Server
	self *busclient.Client
	mon  *quiescence.Monitor

	// jobsFn and peers are nil when those subsystems are off.
	jobsFn func() []jobs.Scheduled
	fed    *federation.Manager

	// askedSeq records, per connection, the bus activity sequence number ITS OWN
	// call to this question produced (see bus.ClientInfo.ActivitySeq).
	//
	// The poller is itself a bus client, so without this the signal would
	// defeat itself: a script checking every minute whether anything is using
	// the machine would show up in the next reading as something using the
	// machine. A connection whose most recent act was asking has therefore
	// done nothing, and only counts again once it does something else.
	//
	// This has to be a sequence number, not a timestamp. Two calls from one
	// connection on a loopback socket, such as an ask immediately followed by
	// real work, routinely land in the same wall-clock millisecond, and
	// bus.ClientInfo.LastActive only has millisecond resolution. A comparison
	// built on that clock cannot tell "asked, then nothing" from "asked, then
	// something, in the same millisecond" apart, and answers the second one
	// wrong. The sequence number is exact regardless of timing.
	mu       sync.Mutex
	askedSeq map[uint64]uint64
	// lastAsk is when anybody last asked, at all. See sampleIdleAfter.
	lastAsk time.Time
}

func newFleetWatcher(srv *bus.Server, self *busclient.Client) *fleetWatcher {
	return &fleetWatcher{
		srv:      srv,
		self:     self,
		mon:      quiescence.NewMonitor(quiescence.Tunables{}),
		askedSeq: map[uint64]uint64{},
	}
}

// noteAsk records that a connection just asked the question. Reading the
// connection's activity sequence back from the bus, rather than stamping our
// own clock, is what makes this exact: it captures the number THIS call
// produced, so any later act on the same connection is guaranteed a different
// one, however close together the two land.
func (w *fleetWatcher) noteAsk(connID uint64) {
	now := time.Now()
	w.mu.Lock()
	w.lastAsk = now
	w.mu.Unlock()
	if connID == 0 {
		return
	}
	if seq, ok := w.srv.ConnActivitySeq(connID); ok {
		w.mu.Lock()
		w.askedSeq[connID] = seq
		w.mu.Unlock()
	}
}

// sampling reports whether anybody is currently using the signal.
func (w *fleetWatcher) sampling(now time.Time) bool {
	w.mu.Lock()
	defer w.mu.Unlock()
	return !w.lastAsk.IsZero() && now.Sub(w.lastAsk) <= sampleIdleAfter
}

// answer is the `fleet.quiescence` handler.
func (w *fleetWatcher) answer(caller bus.CallerIdentity, _ json.RawMessage) (any, error) {
	w.noteAsk(caller.ConnID)
	return w.mon.Latest(), nil
}

// run takes a reading every interval until ctx ends. The dwell is measured
// from these readings rather than from calls, because "nothing has been
// happening for twelve minutes" cannot be established by looking once.
func (w *fleetWatcher) run(ctx context.Context, interval time.Duration) {
	t := time.NewTicker(interval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-t.C:
			if !w.sampling(now) {
				continue
			}
			w.mon.Observe(w.read(ctx))
		}
	}
}

// read gathers one complete reading.
func (w *fleetWatcher) read(ctx context.Context) quiescence.Inputs {
	in := quiescence.Inputs{Now: time.Now()}
	in.Sessions, in.SessionsErr = w.localSessions(ctx)
	in.Clients = w.clients()
	if w.jobsFn != nil {
		for _, j := range w.jobsFn() {
			in.Jobs = append(in.Jobs, quiescence.Job{
				ID: j.ID, Name: j.Name, ActionKind: j.ActionKind,
				NextRun: j.NextRun, Running: j.Running,
			})
		}
	}
	peers, peerSessions := w.peerState(ctx)
	in.Peers = peers
	in.Sessions = append(in.Sessions, peerSessions...)
	return in
}

// localSessions asks whichever provider is live for the fleet. A failure is
// returned rather than swallowed: no answer is not an empty fleet.
func (w *fleetWatcher) localSessions(ctx context.Context) ([]quiescence.Session, error) {
	if w.self == nil {
		return nil, errors.New("no bus client")
	}
	cctx, cancel := context.WithTimeout(ctx, fleetProbeTimeout)
	defer cancel()
	raw, err := w.self.Call(cctx, "sessions.snapshots", map[string]any{})
	if err != nil {
		return nil, err
	}
	return quiescence.ParseSessions("", raw)
}

// clients reduces the live bus connections to the ones that represent somebody
// USING this machine. Providers, plugin sidecars and the hub's own loopback
// client are infrastructure and are dropped by bus.ClientInfo.UserFacing; a
// connection whose most recent act was asking this very question is dropped
// here (see askedSeq) — exactly while its ActivitySeq still matches the one
// its own ask produced, and not a tick longer.
func (w *fleetWatcher) clients() []quiescence.Client {
	live := w.srv.Clients()
	seen := make(map[uint64]bool, len(live))
	var out []quiescence.Client
	w.mu.Lock()
	for _, c := range live {
		seen[c.ConnID] = true
		if !c.UserFacing() {
			continue
		}
		if asked, ok := w.askedSeq[c.ConnID]; ok && asked == c.ActivitySeq {
			continue
		}
		out = append(out, quiescence.Client{Label: c.Label, LastActive: c.LastActive})
	}
	// Forget connections that have gone away, so the map cannot grow without
	// bound across a long uptime of reconnecting clients.
	for id := range w.askedSeq {
		if !seen[id] {
			delete(w.askedSeq, id)
		}
	}
	w.mu.Unlock()
	return out
}

// peerState checks every federated peer. An unreachable peer is recorded as an
// error, which blocks: it is an UNKNOWN peer, not a quiet one.
//
// What is checked is the peer's SESSIONS, not its full predicate. Asking a peer
// for its own fleet.quiescence would recurse — two hubs federated to each other
// would ask each other forever — so this reads the one thing that can be read
// without recursion. The limit is real and worth stating: a peer whose bus
// clients are active, or whose jobs are about to fire, is not detected here.
// The peer's own signal answers that question on the peer's own machine.
func (w *fleetWatcher) peerState(ctx context.Context) ([]quiescence.Peer, []quiescence.Session) {
	if w.fed == nil {
		return nil, nil
	}
	connected := map[string]bool{}
	for _, info := range w.fed.PeersInfo() {
		connected[info.Name] = info.Connected
	}
	var peers []quiescence.Peer
	var sessions []quiescence.Session
	for _, name := range w.fed.Peers() {
		if !connected[name] {
			peers = append(peers, quiescence.Peer{Name: name, Err: "federation link is not connected"})
			continue
		}
		cctx, cancel := context.WithTimeout(ctx, fleetProbeTimeout)
		raw, err := w.fed.Forward(cctx, name, "sessions.snapshots", nil)
		cancel()
		if err != nil {
			peers = append(peers, quiescence.Peer{Name: name, Err: err.Error()})
			continue
		}
		rows, err := quiescence.ParseSessions(name, raw)
		if err != nil {
			peers = append(peers, quiescence.Peer{Name: name, Err: err.Error()})
			continue
		}
		peers = append(peers, quiescence.Peer{Name: name})
		sessions = append(sessions, rows...)
	}
	return peers, sessions
}
