package nodes

import (
	"context"
	"errors"
	"fmt"

	"github.com/djtouchette/workspacer-hub/internal/flyapi"
)

// ErrNotSleepable is returned for a node the hub cannot stop itself. Same
// coordinates and same credential as a wake — see [Node.Wakeable] — but a
// different sentence, because "there is no Connect button for this machine" and
// "there is no off switch for this machine" are different things to be told.
var ErrNotSleepable = errors.New("this node has no cloud coordinates or credential on this hub, so it cannot be put to sleep from here")

// Sleep asks the cloud API to stop a node and returns IMMEDIATELY with the node
// in `stopping`.
//
// It is the exact mirror of [Supervisor.Wake] and it exists because the wake-only
// v1 had a hole with a bill attached: a wake that started a machine and never
// got a provider left that machine RUNNING, and no code path anywhere in the app
// could turn it off again. `wakeFailures` and a detail sentence were the whole
// mitigation. This is the other half.
//
// THREE THINGS ARE DELIBERATELY NOT PARAMETERS, and each is authority the caller
// does not get to declare for itself:
//
//   - The SIGNAL. It is Tunables.StopSignal (SIGTERM), never the caller's. A
//     caller that could name the signal could name SIGKILL, which skips
//     claudemon's flush and denies the entrypoint the chance to write
//     /data/state/last-exit.json — the single artefact that tells the next wake
//     that this stop was deliberate. A stop that destroys its own evidence is
//     not a sleep.
//   - The DRAIN WINDOW. Tunables.StopGrace, sent explicitly on every call
//     because fly.toml's kill_timeout governs a PLATFORM stop and an API stop
//     never reads it.
//   - The COORDINATES. The caller supplies an `id` that SELECTS a row the hub
//     already holds; the app, the machine id, the endpoint and the credential
//     all come from nodes.json. There is no path from a caller's bytes to which
//     machine gets stopped.
//
// It is idempotent per node, for the same reason Wake is: three clients tapping
// the button produce ONE stop call, and three would earn a 429 from a cloud API
// that allows one action per second per machine.
func (s *Supervisor) Sleep(ctx context.Context, id string) (NodeView, error) {
	s.mu.Lock()
	n, ok := s.byID[id]
	if !ok {
		s.mu.Unlock()
		return NodeView{}, fmt.Errorf("%w: %q", ErrUnknownNode, id)
	}
	st := s.st[id]
	switch {
	case st.stopping:
		// Already draining. Spend nothing, and do not re-signal a process that
		// is already handling the first one.
		v := s.viewLocked(n)
		s.mu.Unlock()
		return v, nil
	case st.state == StateStopped:
		// Already off. Spend nothing.
		v := s.viewLocked(n)
		s.mu.Unlock()
		return v, nil
	}
	cli, hasClient := s.clients[id]
	if !hasClient || !n.Wakeable() {
		s.mu.Unlock()
		return NodeView{}, fmt.Errorf("%w: %s", ErrNotSleepable, n.DisplayName())
	}
	// A wake in flight is ABANDONED rather than fought with. Bumping the
	// generation is what makes that safe: the wake's watcher goroutine is still
	// out there, and without this a probe that succeeds one beat after somebody
	// pressed Sleep would report `available` for a machine that is draining.
	// This is also the manual escape hatch for a wake that is going nowhere.
	st.waking = false
	st.gen++
	gen := st.gen
	st.stopping = true
	st.stopDeadline = s.now().Add(s.tun.StopTimeout)
	stopping := s.setLocked(id, StateStopping, "asking the machine to shut down cleanly", upYes)
	view := s.viewLocked(n)
	s.mu.Unlock()
	s.emit(stopping)

	if err := s.stopMachine(ctx, cli, n); err != nil {
		s.mu.Lock()
		if st.gen == gen {
			st.stopping = false
			// The stop was REFUSED, so nothing changed: whatever was running
			// is still running.
			failed := s.setLocked(id, StateUnreachable,
				"the cloud API refused to stop the machine — "+describeFlyError(err)+
					". It may still be running and billing; stop it from the cloud console", upYes)
			v := s.viewLocked(n)
			s.mu.Unlock()
			s.emit(failed)
			return v, err
		}
		v := s.viewLocked(n)
		s.mu.Unlock()
		return v, err
	}

	go s.watchSleep(context.WithoutCancel(ctx), n, cli, gen)
	return view, nil
}

// stopMachine issues the one stop call, with the signal and drain window the
// SUPERVISOR chose. Everything that stops a machine in this package goes
// through here, so there is exactly one place where those two values are
// decided and it is not a call site.
//
// Unlike a start, a stop is NOT retried. A start that fails leaves nothing
// running and retrying costs nothing; a stop that "fails" may well have been
// accepted and be draining right now, and a second signal into a shutting-down
// process tree is how a clean flush becomes a half-written one. The reconcile
// poll is the retry: it sees a machine still `started` with no provider and
// says so, every 30 seconds, forever.
func (s *Supervisor) stopMachine(ctx context.Context, cli flyapi.Client, n Node) error {
	return cli.Stop(ctx, n.Fly.App, n.Fly.MachineID, s.tun.StopSignal, s.tun.StopGrace)
}

// watchSleep follows a stop through to a machine that is actually off.
//
// It uses the cloud API's own blocking wait rather than a poll loop, for the
// same reason watchWake does: reading a machine is rate-limited and a blocking
// wait is not. Unlike a wake, though, the cloud API IS the authority here —
// "the machine is stopped" is a complete answer, where "the machine is started"
// never was.
func (s *Supervisor) watchSleep(ctx context.Context, n Node, cli flyapi.Client, gen int) {
	s.mu.Lock()
	deadline := s.st[n.ID].stopDeadline
	s.mu.Unlock()

	wctx, cancel := context.WithDeadline(ctx, deadline)
	defer cancel()

	err := cli.WaitForState(wctx, n.Fly.App, n.Fly.MachineID, flyapi.StateStopped, s.tun.StopTimeout)

	s.mu.Lock()
	st := s.st[n.ID]
	if st.gen != gen || !st.stopping {
		// Somebody asked for something else while we were waiting. Their
		// transition owns the node now.
		s.mu.Unlock()
		return
	}
	st.stopping = false
	var c *Change
	if err != nil {
		// The stop was ACCEPTED and the machine did not reach `stopped` inside
		// the window. That is the expensive case — it is very likely still
		// running — so it is `unreachable` with the bill named, not `stopped`.
		c = s.setLocked(n.ID, StateUnreachable,
			"the machine was asked to shut down and had not stopped after "+
				s.tun.StopTimeout.String()+" — it may still be running and billing; check the cloud console", upYes)
		s.mu.Unlock()
		s.emit(c)
		s.logf("nodes: %s did not reach stopped within %s after a sleep (%v)", n.ID, s.tun.StopTimeout, err)
		return
	}
	s.markSleptLocked(st)
	c = s.setLocked(n.ID, StateStopped, "", upNo)
	s.mu.Unlock()
	s.emit(c)
	s.logf("nodes: %s is stopped — this hub put it to sleep", n.ID)
}

// markSleptLocked records that the hub itself switched this machine off, and
// drops the exit record it was holding. Caller holds s.mu.
//
// THE DROP IS THE SUBTLE HALF AND IT IS NOT TIDINESS. `lastExit` is read off
// brain.info while a node is UP, and it describes the run BEFORE the one that
// is answering. The moment that run ends, the record the hub holds is two runs
// old — it does not describe the ending that just happened, and rendering it
// beside a machine that has just stopped tells the user about a crash from a
// previous life as though it were news.
//
// Absent is the honest answer, and the contract already says so: a missing
// lastExit means NOBODY KNOWS, not that it ended cleanly. What fills the gap
// here is [NodeView.SleptByHub], which is the hub's own first-hand knowledge
// rather than a record it has fabricated on the node's behalf. The node's own
// account arrives one wake later, and if the two ever disagree the node wins —
// it is the one that was there.
func (s *Supervisor) markSleptLocked(st *state) {
	st.sleptByHub = true
	st.lastExit = nil
}

// stopAfterFailedWake is the automatic half, and the ONLY automatic stop in
// this package.
//
// WHAT CONFINES IT, because an unattended verb that spends and un-spends money
// needs its bounds written down rather than inferred:
//
//  1. It fires only from [Supervisor.failWake] — a wake THIS hub issued, whose
//     window has expired, whose `waking` flag it still holds, and whose
//     generation has not moved. A node somebody else started, or one this hub
//     never touched, never reaches here.
//  2. It fires only when the node's provider NEVER REGISTERED. A node that
//     answered brain.info left `waking` through the success path and cannot
//     arrive here at all. That is the invariant
//     TestTheHubNeverStopsANodeWhoseProviderIsAnswering exists to break.
//  3. It is not on a clock. There is no timer over a working machine anywhere
//     in this package, and adding one is a product decision nobody has made.
//  4. It can be turned off (Tunables.KeepFailedWakesRunning) for the one case
//     where leaving a broken boot up is the point — debugging it.
//
// Best-effort by design: it must never block, and a stop that fails leaves the
// node's detail saying the machine may still be running, which is what the
// wake-only version said in every case.
func (s *Supervisor) stopAfterFailedWake(ctx context.Context, n Node, gen int) {
	if s.tun.KeepFailedWakesRunning {
		return
	}
	cli, ok := s.clients[n.ID]
	if !ok || !n.Wakeable() {
		return
	}
	// THE GUARD, AND IT IS BEFORE THE STOP RATHER THAN AFTER IT.
	//
	// Checking afterwards would still get the bookkeeping right and would still
	// have stopped a machine somebody had just asked for — the window is small
	// and it is exactly the window a user creates by pressing Connect again on
	// a node that is taking too long. A stop is not a write we can roll back.
	s.mu.Lock()
	may := s.mayAutoStopLocked(n.ID, gen)
	s.mu.Unlock()
	if !may {
		return
	}

	sctx, cancel := context.WithTimeout(ctx, s.tun.StopGrace+s.tun.ProbeTimeout)
	defer cancel()
	err := s.stopMachine(sctx, cli, n)

	s.mu.Lock()
	if s.st[n.ID].gen != gen {
		// Something landed while we were asking. It owns the node's state; the
		// stop it raced is reconciled by the poll like any other reading.
		s.mu.Unlock()
		return
	}
	var c *Change
	if err != nil {
		c = s.setLocked(n.ID, StateUnreachable, fmt.Sprintf(
			"the machine was started but its provider did not register within %s, and the hub could not stop it again — "+
				"%s. IT MAY STILL BE RUNNING AND BILLING; stop it from the cloud console",
			s.tun.RegisterTimeout, describeFlyError(err)), upYes)
		s.mu.Unlock()
		s.emit(c)
		s.logf("nodes: %s: the machine did not register after a wake AND could not be stopped (%v) — it may still be billing", n.ID, err)
		return
	}
	// NOT marked sleptByHub: this was not a sleep anybody asked for, and
	// claiming it was would render a node that keeps failing to boot as one
	// somebody deliberately switched off. What the hub knows is that the
	// machine failed to come up and has been stopped again — and wakeFailures,
	// which is NOT cleared, is what keeps the next reconcile calling that
	// `unreachable` rather than a healthy sleeper.
	// upNo, and this is exactly the reading a prose sniff got wrong: this
	// detail says "billing" while describing a machine that is now OFF.
	c = s.setLocked(n.ID, StateUnreachable, fmt.Sprintf(
		"the machine was started but its provider did not register within %s, so the hub STOPPED IT AGAIN "+
			"rather than leave it billing — check the node's boot log", s.tun.RegisterTimeout), upNo)
	s.mu.Unlock()
	s.emit(c)
	s.logf("nodes: %s did not register after a wake; the hub stopped the machine again so it does not keep billing", n.ID)
}

// mayAutoStopLocked is the whole authority of the one unattended stop in this
// package, in one place so it can be broken on purpose. Caller holds s.mu.
//
// Every clause is load-bearing and each covers a different way the automatic
// stop could reach a machine it has no business touching:
//
//   - GENERATION. A wake or a sleep that landed while the watcher was waiting
//     owns this node now. Without this, pressing Connect again on a slow node
//     hands the previous wake's timeout a licence to stop the machine the user
//     just started.
//   - NOT WAKING. A wake in flight is somebody's live intent.
//   - NOT STOPPING. A sleep already has the machine; do not double-signal a
//     draining process tree.
//   - NOT AVAILABLE. THE ONE THAT MATTERS MOST: a node whose provider is
//     answering is a node somebody may be typing at. Nothing automatic in this
//     hub may switch that off, for any reason, ever. It is unreachable through
//     the normal flow — an available node left `waking` through watchWake's
//     success path and never calls failWake — and it is asserted here anyway,
//     because "the call path cannot reach it" is a claim about today's code and
//     this is a claim about the machine.
//
// TestTheHubNeverStopsANodeWhoseProviderIsAnswering breaks the last two.
func (s *Supervisor) mayAutoStopLocked(id string, gen int) bool {
	st := s.st[id]
	if st == nil {
		return false
	}
	return st.gen == gen && !st.waking && !st.stopping && st.state != StateAvailable
}
