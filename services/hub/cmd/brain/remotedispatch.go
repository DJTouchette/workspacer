package main

// REMOTE WORKER DISPATCH — the PEER half: a worker whose manager lives on
// another machine, and the return channel that gets its report home.
//
// ORIGIN HALF: services/hub/internal/bus/remotedispatch.go (which stamps the
// provenance) and apps/desktop/src/main/services/remoteDispatchRegistry.ts
// (which validates the answer and injects the wake). Read the bus file first —
// it states the problem and why the router is where the stamp is minted.
//
// WHAT THIS SIDE OWES, and it is deliberately almost nothing:
//
//  1. RECORD the opaque dispatch id that arrived on agents.spawn, against the
//     session it created. Nothing else about the origin is stored, because
//     nothing else was sent: the origin does not tell a peer which manager to
//     wake, which machine it is, or what its filesystem looks like. The peer
//     acknowledges an id it was given; it never names a recipient.
//  2. EMIT one `agent.dispatch.update` per reportable event for that worker —
//     progress, blocked, and the terminal finished/escalated — carrying the
//     SAME fleetEntry the local wake path would have composed. The origin
//     renders the text with its own buildFleetMessage, so the `[supervisor]`
//     wire format has exactly one authority and a cross-machine wake reads
//     identically to a local one.
//  3. REPLAY the terminal update on demand, so an origin whose link was down
//     when the worker finished can reconcile on reconnect instead of polling.
//
// WHY THE ENTRY AND NOT THE TEXT. buildFleetMessage exists on both sides, so
// either would work — but the headers are parsed, not merely displayed
// (fleetmsg.go, main/shared/fleetMessages.ts), and a peer composing final text
// would make every future header change a synchronized two-machine deploy. An
// entry is data; the sentence is the origin's.
//
// WHY THE UPDATE IS AN EVENT AND NOT A CALL. The origin is a desktop behind
// NAT: it dials out to this node, it is not dialable. The federation link it
// already holds is subscribed to `agent.*`, so publishing on that namespace
// reaches it over the connection it opened, with the peer's name stamped on the
// envelope by the origin's own link — which is what makes "this came from the
// machine I dispatched to" a fact the origin verified rather than a claim the
// payload made. No inbound listener, no polling manager.
//
// WHAT IS NOT PROMISED. Not exactly-once delivery: a websocket is not a queue.
// Every update carries a per-dispatch monotonic `seq`, the origin dedups on
// (dispatchId, seq), and a terminal update is replayable — that is at-least-once
// with an exactly-once EFFECT, which is the honest description.

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
)

// remoteOriginParam is the provenance block the origin's router stamped onto
// agents.spawn. It is accepted ONLY because the origin hub's federation link is
// the only caller the peer's own sanitizeSpawnParams lets keep the field.
type remoteOriginParam struct {
	Protocol   int    `json:"protocol"`
	DispatchID string `json:"dispatchId"`
	OwnerKey   string `json:"ownerKey"`
}

// dispatchUpdateKind is the vocabulary of the return channel, deliberately a
// SUBSET of FleetMessageKind: the origin maps each to the header its own
// builder already owns. `catch-up` and `threshold` are absent — the first is
// the origin's own backstop and the second is armed and evaluated where the
// watcher lives.
const (
	dispatchKindFinished  = "worker-finished"
	dispatchKindEscalated = "worker-escalated"
	dispatchKindBlocked   = "blocked"
	dispatchKindProgress  = "progress"
)

// dispatchUpdate is the payload of `agent.dispatch.update`. Field-for-field the
// TS twin is RemoteDispatchUpdate in remoteDispatchRegistry.ts.
type dispatchUpdate struct {
	Protocol   int    `json:"protocol"`
	DispatchID string `json:"dispatchId"`
	Kind       string `json:"kind"`
	// SessionID is the WORKER's id on this node. The origin cross-checks it
	// against the id its own spawn result recorded, so a dispatch id that leaked
	// to another session here still cannot report as that dispatch.
	SessionID string `json:"sessionId"`
	// Seq is monotonic per dispatch, from 1. The origin's dedup key together
	// with DispatchID.
	Seq int64 `json:"seq"`
	// Ts is when this node produced the update (unix ms), for the operator's
	// record. Never used for ordering — Seq is.
	Ts int64 `json:"ts"`
	// Final marks the terminal update: the one a replay re-sends and the one
	// that closes the origin's record.
	Final bool `json:"final"`
	// Entry is the ordinary fleet bullet, composed here by the same code the
	// local wake path uses.
	Entry fleetEntry `json:"entry"`
}

// remoteDispatch is one open dispatch this node is executing for another hub.
type remoteDispatch struct {
	dispatchID string
	sessionID  string
	seq        int64
	// last is the latest durable update, including a block or progress report
	// while the worker is running. A final update closes its sequence.
	last           *dispatchUpdate
	lease          *dispatchLease
	acknowledgedAt int64
}

// remoteDispatchStore journals admission leases and the latest progress/block/
// terminal update. A brain restart cannot make a claimed lease reusable or
// discard a result whose origin was disconnected.
type remoteDispatchStore struct {
	mu   sync.Mutex
	file string
	m    map[string]*remoteDispatch // dispatchId -> record
	// bySession is the reverse index the wake paths hit on every transition, so
	// the common case (a worker with no remote origin at all) is one map read.
	bySession map[string]string // sessionId -> dispatchId
}

func newRemoteDispatchStore() *remoteDispatchStore {
	return &remoteDispatchStore{m: map[string]*remoteDispatch{}, bySession: map[string]string{}}
}

func (s *remoteDispatchStore) record(dispatchID, sessionID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	d := s.m[dispatchID]
	if d == nil {
		d = &remoteDispatch{dispatchID: dispatchID}
		s.m[dispatchID] = d
	}
	d.sessionID = sessionID
	s.bySession[sessionID] = dispatchID
	return s.persistLocked()
}

func (s *remoteDispatchStore) forSession(sessionID string) string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.bySession[sessionID]
}

// next allocates the next seq for a dispatch and records a terminal update.
// Returns ok=false for an unknown dispatch, which is how a race with agents.close
// declines to publish rather than inventing a record.
func (s *remoteDispatchStore) next(dispatchID string, final bool) (int64, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	d, ok := s.m[dispatchID]
	if !ok || (d.last != nil && d.last.Final) {
		return 0, false
	}
	d.seq++
	return d.seq, true
}

func (s *remoteDispatchStore) keepFinal(u dispatchUpdate) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if d, ok := s.m[u.DispatchID]; ok {
		copyOf := u
		d.last = &copyOf
		return s.persistLocked() == nil
	}
	return false
}

func (s *remoteDispatchStore) replay(dispatchID string) (*dispatchUpdate, string, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	d, ok := s.m[dispatchID]
	if !ok {
		return nil, "", false
	}
	return d.last, d.sessionID, true
}

// forget drops a dispatch when its session is forgotten (agents.close), so the
// two maps do not retain an entry per session for the process lifetime — the
// same concern finishWatcher.forgetWorker has.
func (s *remoteDispatchStore) forget(sessionID string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	id, ok := s.bySession[sessionID]
	if !ok {
		return
	}
	delete(s.bySession, sessionID)
	// Retain the durable result after a session card is closed.
	_ = id
}

// ── spawn-time admission ────────────────────────────────────────────────────

// acceptRemoteOrigin validates the provenance block on an incoming spawn and
// returns the dispatch id to record, or an error that REFUSES THE SPAWN.
//
// Refusing is the point. Every failure here is a case where the worker would
// start and its report would have nowhere to go — an orphan running on someone
// else's machine, burning that account's tokens, invisible to the manager that
// asked for it. A loud refusal at spawn time is strictly better than a silent
// one at finish time, and it is what makes version skew diagnosable: an origin
// stamping protocol 2 against this build gets a sentence naming both numbers.
func (r *registry) acceptRemoteOrigin(p spawnParams) (string, error) {
	if p.RemoteOrigin == nil {
		return "", nil
	}
	o := p.RemoteOrigin
	if o.Protocol != bus.DispatchProtocol {
		return "", fmt.Errorf("agents.spawn: this node speaks remote-dispatch protocol %d and the dispatching machine sent %d — "+
			"upgrade the older of the two workspacer installs; the spawn was refused rather than started with no way to report back",
			bus.DispatchProtocol, o.Protocol)
	}
	if !bus.ValidDispatchID(o.DispatchID) {
		return "", fmt.Errorf("agents.spawn: remoteOrigin.dispatchId is malformed")
	}
	if r.remote == nil || r.publish == nil || r.store == nil {
		// Catalog scope: no live session store, no finish watcher, nothing that
		// could ever emit a callback. Say so instead of starting the worker.
		return "", fmt.Errorf("agents.spawn: this node is registered in catalog scope and cannot execute dispatched work — " +
			"it has no live session store and would never report back. Run `workspacer serve` in full scope on the target machine.")
	}
	return o.DispatchID, nil
}

// remoteDispatchID answers "was this worker dispatched from another hub?" for
// the wake paths. Empty for every ordinary session, which is the fast path.
func (r *registry) remoteDispatchID(sessionID string) string {
	if r.remote == nil || sessionID == "" {
		return ""
	}
	return r.remote.forSession(sessionID)
}

// emitDispatchUpdate publishes one callback onto this node's bus, where the
// origin's federation link picks it up.
//
// Best-effort by construction, and honestly so: publish is a fire-and-forget
// websocket write, so a link that is down loses the event. That is exactly what
// `final` + dispatchReplay exist for — the origin re-asks on reconnect for every
// dispatch it still holds open.
func (r *registry) emitDispatchUpdate(dispatchID, kind, sessionID string, entry fleetEntry, final bool) bool {
	if r.remote == nil || r.publish == nil || dispatchID == "" {
		return false
	}
	seq, ok := r.remote.next(dispatchID, final)
	if !ok {
		return false
	}
	u := dispatchUpdate{
		Protocol:   bus.DispatchProtocol,
		DispatchID: dispatchID,
		Kind:       kind,
		SessionID:  sessionID,
		Seq:        seq,
		Ts:         time.Now().UnixMilli(),
		Final:      final,
		Entry:      entry,
	}
	if !r.remote.keepFinal(u) {
		return false
	}
	data, err := json.Marshal(u)
	if err != nil {
		return false
	}
	r.publish(bus.TopicDispatchUpdate, data)
	return true
}

// ── agents.dispatchReplay ───────────────────────────────────────────────────

// dispatchReplay re-publishes the terminal update for one dispatch. The origin
// calls it, over the federation link, for each dispatch it still believes open
// when a peer link comes back up.
//
// It is a RE-SEND, never a re-derivation: it publishes the update this node
// already produced, with its original seq, so the origin's dedup makes a replay
// of an update that DID arrive a no-op. That is the whole reason reconnect
// reconciliation is bounded and safe rather than a duplicate-wake generator.
//
// Three distinguishable answers, because the origin acts differently on each:
//
//   - state "running": the dispatch is known and has not finished. Keep waiting.
//   - state "replayed": a terminal update was re-published; it is on its way.
//   - state "unknown": this node has no record. Either it restarted (losing the
//     worker with it) or the dispatch was never accepted here. The origin
//     TOMBSTONES rather than waits — a lost dispatch that is visibly lost is
//     recoverable; one that is silently pending is not.
//
// OPERATOR-ONLY by construction, like the rest of the fleet verbs: it is in
// neither scoped tier's exact-name allowlist. It discloses only whether an id
// the caller already holds is known here.
func (r *registry) dispatchReplay(_ context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		DispatchID string `json:"dispatchId"`
		OriginKey  string `json:"originKey"`
		AckedSeq   int64  `json:"ackedSeq"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if !bus.ValidDispatchID(p.DispatchID) {
		return nil, fmt.Errorf("agents.dispatchReplay: dispatchId is malformed")
	}
	if r.remote == nil {
		return jsonResult(map[string]any{"state": "unknown", "dispatchId": p.DispatchID})
	}
	r.remote.mu.Lock()
	d := r.remote.m[p.DispatchID]
	authorized := d != nil && d.lease != nil && d.lease.Owner == p.OriginKey
	r.remote.mu.Unlock()
	if !authorized {
		return nil, fmt.Errorf("dispatch unavailable for this connection")
	}
	if p.AckedSeq > 0 {
		r.remote.mu.Lock()
		d := r.remote.m[p.DispatchID]
		if d == nil || d.last == nil || !d.last.Final || d.last.Seq != p.AckedSeq {
			r.remote.mu.Unlock()
			return nil, fmt.Errorf("terminal acknowledgement does not match")
		}
		d.acknowledgedAt = time.Now().UnixMilli()
		err := r.remote.persistLocked()
		r.remote.mu.Unlock()
		if err != nil {
			return nil, err
		}
		return jsonResult(map[string]any{"state": "acknowledged"})
	}
	last, sessionID, known := r.remote.replay(p.DispatchID)
	switch {
	case !known:
		return jsonResult(map[string]any{"state": "unknown", "dispatchId": p.DispatchID})
	case last == nil:
		return jsonResult(map[string]any{"state": "running", "dispatchId": p.DispatchID, "sessionId": sessionID})
	}
	if last.Kind == dispatchKindBlocked {
		current, exists := findFleetSession(r.fleetSessions(context.Background()), sessionID)
		if exists && !isBlockedAmbient(current.AmbientState) {
			return jsonResult(map[string]any{"state": "running", "dispatchId": p.DispatchID, "sessionId": sessionID})
		}
	}
	if data, err := json.Marshal(*last); err == nil && r.publish != nil {
		r.publish(bus.TopicDispatchUpdate, data)
	}
	return jsonResult(map[string]any{
		"state": "replayed", "dispatchId": p.DispatchID, "sessionId": sessionID,
		"kind": last.Kind, "seq": last.Seq,
	})
}

// ── fleet.dispatchCapabilities ──────────────────────────────────────────────

// dispatchCapabilities is the peer-side READINESS ANSWER a manager's machine
// asks for before it dispatches anything: does this node support the return
// channel at all, which harnesses are actually usable HERE, and which
// directories on THIS filesystem are real repositories to work in.
//
// EVERY FIELD IS MEASURED ON THIS MACHINE, which is the point. The origin
// cannot infer a peer's Claude login from its own, cannot translate a local
// path into a remote one, and must not present a provider as available because
// the desktop that is asking happens to have it. A node with Codex installed
// but not logged in reports exactly that, and the manager gets to choose
// something that will work instead of discovering it in a dead worker.
//
// It names no credential and no address: provider ids, a boolean, a sentence,
// and directories that are already on every agents.list row's `cwd`.
func (r *registry) dispatchCapabilities(ctx context.Context, _ json.RawMessage) (json.RawMessage, error) {
	out := map[string]any{
		"protocol": bus.DispatchProtocol,
		"handoff":  map[string]any{"version": 1, "receiptVersion": 1, "transport": "git-remote", "objectFormats": []string{"sha1", "sha256"}, "chunkBytes": 256 << 10, "fileBytes": 16 << 20, "taskBytes": 64 << 20, "files": 128},
		// executes is the honest single bit a client should gate its UI on: can
		// this node run dispatched work AND report back? Catalog scope is the
		// case that answers false while everything else looks healthy.
		"executes": r.remote != nil && r.publish != nil && r.store != nil,
		"scope":    r.scope,
	}
	if r.remote == nil || r.store == nil {
		out["unsupportedReason"] = "this node is registered in catalog scope: it has no live session store, so a dispatched worker could never report back"
	}
	out["providers"] = r.dispatchProviderReadiness(ctx)
	out["cwds"] = r.dispatchCwdChoices(ctx)
	return jsonResult(out)
}
