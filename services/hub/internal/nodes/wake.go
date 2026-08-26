package nodes

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/flyapi"
)

// ErrNotWakeable is returned for a node the hub cannot start itself.
var ErrNotWakeable = errors.New("this node has no cloud coordinates or credential on this hub, so it cannot be woken from here")

// Wake asks the cloud API to start a node and returns IMMEDIATELY with the
// node in `waking`.
//
// It is asynchronous on purpose. The whole reason this feature exists is that
// the four-state model makes the wait honest: a client renders `waking` with a
// disabled composer and a "ready in ~20s" line, rather than a spinner on a
// blocked request. (It is also why there is no queue for input typed during a
// wake — a durable, ordered, idempotent dispatch queue is the most expensive
// thing in the brief, and a disabled composer is both cheaper and more honest
// than a prompt that might replay twice.)
//
// It is idempotent per node. Three clients tapping the button produce ONE
// start call: a wake already in flight is reported back as-is. That is not
// only tidiness — the cloud API allows one action per second per machine, so
// three concurrent starts is a 429, and a 429 on the wake path reads to a
// user as "the button does nothing".
func (s *Supervisor) Wake(ctx context.Context, id string) (NodeView, error) {
	s.mu.Lock()
	n, ok := s.byID[id]
	if !ok {
		s.mu.Unlock()
		return NodeView{}, fmt.Errorf("%w: %q", ErrUnknownNode, id)
	}
	st := s.st[id]
	switch {
	case st.waking:
		v := s.viewLocked(n)
		s.mu.Unlock()
		return v, nil
	case st.state == StateAvailable:
		// Already up. Spend nothing.
		v := s.viewLocked(n)
		s.mu.Unlock()
		return v, nil
	}
	cli, hasClient := s.clients[id]
	if !hasClient || !n.Wakeable() {
		s.mu.Unlock()
		return NodeView{}, fmt.Errorf("%w: %s", ErrNotWakeable, n.DisplayName())
	}
	// Claim the wake before releasing the lock so a second caller landing in
	// the same millisecond is folded into this one rather than starting a
	// second machine start.
	//
	// The generation bump abandons any sleep watcher still in flight for the
	// same reason Sleep abandons a wake: two transitions cannot both own a
	// node, and the one somebody just asked for wins.
	st.stopping = false
	st.gen++
	gen := st.gen
	st.waking = true
	// Whatever this machine's last stop was, it is no longer the current
	// answer to "why is this off" — the hub is turning it on.
	st.sleptByHub = false
	st.wakeDeadline = s.now().Add(s.tun.RegisterTimeout)
	waking := s.setLocked(id, StateWaking, "starting the machine", upYes)
	view := s.viewLocked(n)
	s.mu.Unlock()
	s.emit(waking)

	// THE ZOMBIE PRE-FLIGHT, and its placement is the point: BEFORE the start
	// call, not after.
	//
	// If a dead provider is still holding the capability slot, the machine we
	// are about to boot will dial in, re-register, and be REFUSED — it will be
	// up and provide nothing, which is the single most likely way this feature
	// disappoints. Clearing the slot now, while the machine is still off, wins
	// the race by seconds rather than by luck. The periodic poll would get
	// there eventually; "eventually" is not good enough on the one code path
	// where a boot is imminent.
	s.evictIfSilent(ctx)

	if err := s.startWithRetry(ctx, cli, n); err != nil {
		s.mu.Lock()
		if st.gen != gen {
			v := s.viewLocked(n)
			s.mu.Unlock()
			return v, err
		}
		st.waking = false
		st.wakeFailures++
		// NOTHING TO STOP HERE, and that is why this path does not try. The
		// cloud API refused the start, so no machine was started and there is
		// no bill to close. The failed-wake stop belongs to failWake, where a
		// machine really is up.
		failed := s.setLocked(id, StateUnreachable, "the cloud API refused to start the machine — "+describeFlyError(err), upNo)
		v := s.viewLocked(n)
		s.mu.Unlock()
		s.emit(failed)
		return v, err
	}

	go s.watchWake(context.WithoutCancel(ctx), n, cli, gen)
	return view, nil
}

// evictIfSilent probes the brain and drops the provider if one is registered
// and does not answer. Best-effort by design: it must never block a wake.
func (s *Supervisor) evictIfSilent(ctx context.Context) {
	if !s.bus.BrainProviderRegistered() {
		return
	}
	pctx, cancel := context.WithTimeout(ctx, s.tun.ProbeTimeout)
	defer cancel()
	switch _, err := s.bus.ProbeBrain(pctx); {
	case err == nil:
		return // somebody live is on the other end; leave them alone
	case errors.Is(err, ErrNoProvider):
		return // nothing registered; no slot to free
	case errors.Is(err, ErrProbeUnavailable):
		return // the HUB could not ask; never evict on our own failure
	}
	if s.bus.EvictBrainProvider() {
		s.logf("nodes: wake pre-flight evicted a registered brain provider that did not answer brain.info — " +
			"it would have refused the woken node's re-registration")
	}
}

// startWithRetry issues the start, retrying transient failures once.
//
// A 404 is not retried: the app or machine id in nodes.json is wrong and
// asking again will be wrong again. A 429 is not retried either — the correct
// answer to a rate limit is to stop asking, and a wake already in flight is
// reported back to the next caller rather than re-issued.
func (s *Supervisor) startWithRetry(ctx context.Context, cli flyapi.Client, n Node) error {
	var err error
	for attempt := 0; attempt <= s.tun.StartRetries; attempt++ {
		if attempt > 0 {
			t := time.NewTimer(s.tun.StartRetryDelay)
			select {
			case <-ctx.Done():
				t.Stop()
				return ctx.Err()
			case <-t.C:
			}
		}
		err = cli.Start(ctx, n.Fly.App, n.Fly.MachineID)
		if err == nil {
			return nil
		}
		var apiErr *flyapi.APIError
		if errors.As(err, &apiErr) && apiErr.NotFound() {
			return err
		}
		var rl *flyapi.RateLimitError
		if errors.As(err, &rl) {
			return err
		}
		s.logf("nodes: start for %s failed (%v)%s", n.ID, err, retrySuffix(attempt, s.tun.StartRetries))
	}
	return err
}

func retrySuffix(attempt, max int) string {
	if attempt < max {
		return "; retrying once"
	}
	return ""
}

// watchWake follows a wake through to a working node, or to an honest failure.
//
// Two stages, and the first uses the cloud API's OWN wait endpoint rather than
// a poll loop: reading a machine's state is rate-limited at 5/s and a blocking
// wait is not rate-limited at all, so one call replaces the loop and cannot
// earn a 429. The second stage is the only one that matters to a user, though:
// a machine being `started` says nothing about whether workspacer is running
// on it. The node is not ready until its BRAIN answers.
func (s *Supervisor) watchWake(ctx context.Context, n Node, cli flyapi.Client, gen int) {
	s.mu.Lock()
	deadline := s.st[n.ID].wakeDeadline
	s.mu.Unlock()

	wctx, cancel := context.WithDeadline(ctx, deadline)
	defer cancel()

	if err := cli.WaitForState(wctx, n.Fly.App, n.Fly.MachineID, flyapi.StateStarted, s.tun.RegisterTimeout); err != nil {
		// Not fatal: the machine may already have been started (a wait can
		// race a boot that finished first), and the authority on readiness is
		// the brain, not the cloud API. Note it and keep waiting for the one
		// signal that actually means something.
		s.logf("nodes: %s: waiting for the machine to start returned %v; falling back to the provider probe", n.ID, err)
	} else {
		s.mu.Lock()
		if s.st[n.ID].gen != gen {
			s.mu.Unlock()
			return
		}
		booted := s.setLocked(n.ID, StateWaking, "the machine is up; waiting for its provider to register", upYes)
		s.mu.Unlock()
		s.emit(booted)
	}

	t := time.NewTicker(s.tun.RegisterPollInterval)
	defer t.Stop()
	for {
		pctx, pcancel := context.WithTimeout(wctx, s.tun.ProbeTimeout)
		pr, err := s.bus.ProbeBrain(pctx)
		pcancel()
		// ATTRIBUTION IS THE WHOLE CHECK, and there is no anonymous shortcut
		// beside it. This condition used to read
		//
		//	err == nil && (s.attribute(pr.NodeID) == n.ID || pr.NodeID == "")
		//
		// and that trailing clause was load-bearing in EXACTLY the case where
		// it was wrong. With one node registered `attribute("")` already
		// returns that node's id, so the clause changed nothing; with two, it
		// let ANY anonymous brain — including one belonging to a node that was
		// already up — end this node's wake. The result was the node marked
		// `available` with `wakeFailures` reset to 0 and the other node's exit
		// record copied onto its row, for a machine that may never have
		// booted. wakeFailures is this arc's stated mitigation for a node that
		// keeps failing to boot, and that path RESET it.
		//
		// The consequence, stated plainly because it is a real cost: a
		// multi-node deployment whose nodes do not set WKS_NODE_ID can no
		// longer complete a wake at all — the watcher will time out and
		// failWake will stop the machine again. That is the same refusal
		// Reconcile has always made (see attribute's own comment), arriving on
		// the path that spends money, and the fix for it is to set
		// WKS_NODE_ID: deploy/fly/node/fly.toml's [env] now does.
		if err == nil && s.attribute(pr.NodeID) == n.ID {
			s.mu.Lock()
			st := s.st[n.ID]
			// SOMEBODY ELSE OWNS THIS NODE NOW — most likely a Sleep pressed
			// mid-wake. Reporting `available` here would put a machine that is
			// draining on screen as connected, and a composer would open at it.
			if st.gen != gen {
				s.mu.Unlock()
				return
			}
			st.waking = false
			st.wakeFailures = 0
			st.lastSeen = s.now()
			// The wake succeeded, so the node can now say how the run BEFORE
			// this one ended — which is the only account anybody has of the
			// stop the hub just reversed.
			detail := ""
			if pr.LastExit != nil {
				st.lastExit = pr.LastExit
				if !pr.LastExit.Clean() {
					detail = pr.LastExit.Describe()
					s.logf("nodes: %s woke, and reports that its previous run ended %q", n.ID, pr.LastExit.Reason)
				}
			}
			up := s.setLocked(n.ID, StateAvailable, detail, upYes)
			s.mu.Unlock()
			s.emit(up)
			s.logf("nodes: %s is available", n.ID)
			return
		}
		select {
		case <-wctx.Done():
			s.failWake(ctx, n, gen)
			return
		case <-t.C:
		}
	}
}

// failWake records a wake that started a machine but never got a provider —
// and then STOPS THAT MACHINE, which is what the sleep path exists to make
// possible.
//
// This is the hole the wake-only v1 shipped with, named in its own contract as
// a known cost: the hub started a billable machine, the machine never became
// usable, and nothing in the app could turn it off again. `wakeFailures` and a
// detail sentence reading "it may still be running" were the entire mitigation,
// and with no public IP there is not even ambient traffic to notice it. It just
// billed until somebody opened the cloud console.
//
// THE THREE GUARDS THAT KEEP THIS FROM BEING AN IDLE TIMER, all checked here
// under the lock before anything is stopped:
//
//   - `st.waking` must still be set. Only a wake this hub issued and is still
//     tracking can end here.
//   - `st.gen` must not have moved. A wake or a sleep that landed while the
//     watcher was waiting owns the node instead.
//   - the node's provider must NEVER HAVE REGISTERED. A node that answered
//     brain.info left `waking` through watchWake's success path and cannot
//     reach this function at all — which is the invariant
//     TestTheHubNeverStopsANodeWhoseProviderIsAnswering breaks on purpose.
//
// There is no clock over a working machine anywhere in this package, and there
// must not be: powering a machine down because nobody used it for a while is a
// product decision nobody has made.
func (s *Supervisor) failWake(ctx context.Context, n Node, gen int) {
	s.mu.Lock()
	st := s.st[n.ID]
	if !st.waking || st.gen != gen {
		s.mu.Unlock()
		return
	}
	st.waking = false
	st.wakeFailures++
	failures := st.wakeFailures
	// STILL UP at this instant: the stop has not been issued yet, and
	// stopAfterFailedWake is what settles it either way a moment from now.
	c := s.setLocked(n.ID, StateUnreachable, fmt.Sprintf(
		"the machine was started but its provider did not register within %s — the hub is stopping it again; "+
			"check the node's boot log", s.tun.RegisterTimeout), upYes)
	s.mu.Unlock()
	s.emit(c)
	s.logf("nodes: %s did not register within %s after a wake (%d consecutive failures)",
		n.ID, s.tun.RegisterTimeout, failures)
	s.stopAfterFailedWake(ctx, n, gen)
}
