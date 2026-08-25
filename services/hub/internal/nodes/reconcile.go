package nodes

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/flyapi"
)

// Run reconciles once and then keeps a reading every PollInterval until ctx
// ends.
//
// UNLIKE fleet.quiescence's sampler, this poll does NOT wind down when nobody
// is asking. Three of its jobs need it to be always on: it is the liveness
// signal that turns `waking` into `available`, it is the only thing that ever
// evicts a zombie provider, and a registry whose belief is only true while
// somebody is watching is the exact "do not trust in-memory state" failure the
// brief warns about. The cost is one no-argument brain.info call every 30
// seconds, and it is only paid at all when a nodes.json exists — which is to
// say, on a deployment that has a remote node.
func (s *Supervisor) Run(ctx context.Context) {
	if len(s.nodes) == 0 {
		return
	}
	s.Reconcile(ctx)
	t := time.NewTicker(s.tun.PollInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			s.Reconcile(ctx)
		}
	}
}

// Reconcile takes one complete reading and folds it into every node's state.
//
// THIS IS ALSO THE HUB-RESTART PATH. The hub keeps no node state on disk, so
// "reconcile on boot" is not a special case — it is this function, called once
// before the poll loop starts. A hub that comes back up while a machine is
// mid-boot finds it `started` with no provider and says so; one that comes
// back up beside a running node finds its provider answering and says
// `available`; one that comes back beside a stopped machine says `stopped`.
// None of that depends on anything the previous process believed.
func (s *Supervisor) Reconcile(ctx context.Context) {
	pr, attachedTo, silent, asked := s.probe(ctx)
	if !asked {
		// The hub could not ask. NO READING, NO UPDATE — a state derived from
		// "we failed to look" is worse than a stale one, and it is exactly how
		// a healthy node gets rendered unreachable during a blip in the hub's
		// own loopback client.
		s.logf("nodes: skipped a reading — the hub's own bus client could not ask %s", probeMethodName)
		return
	}

	s.mu.Lock()
	s.reconciled = true
	var changes []*Change

	// The zombie half. A provider that is REGISTERED and does not answer is
	// the failure mode that makes a wake silently do nothing: it holds the
	// capability slot, and the woken node's re-registration is refused by the
	// bus's first-registration-wins guard. Nothing else in the hub will ever
	// notice — the brain subscribes to no topic, so no write to that socket
	// ever fails, and there is no server-side ping or read deadline.
	//
	// Two strikes, then evict, so one slow answer under load does not throw a
	// live node off the bus.
	if silent {
		s.silentStrikes++
		if s.silentStrikes >= s.tun.SilentStrikes {
			s.silentStrikes = 0
			s.mu.Unlock()
			if s.bus.EvictBrainProvider() {
				s.logf("nodes: evicted a registered brain provider that stopped answering brain.info — " +
					"it was holding the capability slot a woken node needs")
			}
			s.mu.Lock()
		}
	} else {
		s.silentStrikes = 0
	}

	for _, n := range s.nodes {
		st := s.st[n.ID]
		// A sleep in flight owns this node's state until it finishes or times
		// out, exactly as a wake does. Without this the poll would find the
		// machine still `started` mid-drain and call it unreachable, turning a
		// deliberate shutdown into a warning on the user's screen.
		if st.stopping {
			if s.now().Before(st.stopDeadline) {
				continue
			}
			// The watcher should have cleared this. If it did not, do not
			// leave a node parked in `stopping` forever — and say the
			// expensive thing, because a stop that never completed means a
			// machine that is probably still billing.
			st.stopping = false
			changes = append(changes, s.setLocked(n.ID, StateUnreachable,
				"the sleep timed out: the machine was asked to shut down and never reported stopped — "+
					"it may still be running and billing; check the cloud console", upYes))
			continue
		}
		// A wake in flight owns this node's state until it finishes or times
		// out; the poll must not race it back to `stopped` between the start
		// call and the boot.
		if st.waking {
			if s.now().Before(st.wakeDeadline) {
				continue
			}
			// The watcher should have cleared this. If it did not (the hub
			// was busy, or the watcher's context died), do not leave a node
			// parked in `waking` forever.
			st.waking = false
			st.wakeFailures++
			changes = append(changes, s.setLocked(n.ID, StateUnreachable,
				"the wake timed out: the machine was started but its provider never registered", upYes))
			continue
		}
		if attachedTo == n.ID {
			st.lastSeen = s.now()
			st.wakeFailures = 0
			if pr.LastExit != nil {
				if st.lastExit == nil || *st.lastExit != *pr.LastExit {
					if !pr.LastExit.Clean() {
						s.logf("nodes: %s reports that its previous run ended %q — it failed rather than being put to sleep",
							n.ID, pr.LastExit.Reason)
					}
				}
				st.lastExit = pr.LastExit
			}
			// A node that is UP but whose previous run ended badly is still
			// available — it is working now. What it is not is unremarkable,
			// so the detail says so: this is the only moment the hub can see
			// a crash that happened while it was not watching.
			detail := ""
			if !st.lastExit.Clean() && st.lastExit != nil {
				detail = st.lastExit.Describe()
			}
			changes = append(changes, s.setLocked(n.ID, StateAvailable, detail, upYes))
			continue
		}
		changes = append(changes, s.reconcileOneLocked(ctx, n))
	}
	s.mu.Unlock()
	s.emit(changes...)
}

// probe asks the bus who is attached.
//
// It returns the node id the answering brain named, and whether a provider is
// registered but SILENT. The three outcomes are deliberately distinct:
//
//   - answered             → a brain is attached; attachedTo names which node.
//   - ErrNoProvider        → nothing is attached. Ordinary, and what a stopped
//     node looks like.
//   - registered, silent   → the zombie. Not an answer, and not an absence.
//   - ErrProbeUnavailable  → the HUB could not ask. asked=false, and the
//     caller must not update anything from it.
func (s *Supervisor) probe(ctx context.Context) (pr Probe, attachedTo string, silent, asked bool) {
	pctx, cancel := context.WithTimeout(ctx, s.tun.ProbeTimeout)
	defer cancel()
	pr, err := s.bus.ProbeBrain(pctx)
	switch {
	case err == nil:
		return pr, s.attribute(pr.NodeID), false, true
	case errors.Is(err, ErrNoProvider):
		return Probe{}, "", false, true
	case errors.Is(err, ErrProbeUnavailable):
		// Our own fault. Not an answer, and not an accusation.
		return Probe{}, "", false, false
	default:
		// A timeout with nobody registered is not a zombie; there is no slot
		// to free and nothing to accuse.
		return Probe{}, "", s.bus.BrainProviderRegistered(), true
	}
}

// probeMethodName is only used in log lines; the probe itself is the caller's,
// behind BusProbe.
const probeMethodName = "brain.info"

// attribute decides WHICH node an answering brain is.
//
// A brain that names itself (WKS_NODE_ID on the node, surfaced in brain.info)
// is believed, if the name is in the registry. Otherwise: with exactly one
// registered node there is only one possible answer, so say it. With several,
// say nothing rather than guess — under provider-attach only one brain can own
// the capability slot at a time anyway, so "one of them is up and it did not
// say which" is the truth, and a wrong guess would render a stopped machine as
// available.
func (s *Supervisor) attribute(named string) string {
	if named != "" {
		if _, ok := s.byID[named]; ok {
			return named
		}
		s.logf("nodes: a brain registered naming node %q, which is not in nodes.json — ignoring the claim", named)
	}
	if len(s.nodes) == 1 {
		return s.nodes[0].ID
	}
	return ""
}

// reconcileOneLocked derives one non-attached node's state from the cloud API.
// Caller holds s.mu.
func (s *Supervisor) reconcileOneLocked(ctx context.Context, n Node) *Change {
	cli, ok := s.clients[n.ID]
	if !ok {
		// NO CREDENTIAL, SO NO OPINION. The hub cannot distinguish "off on
		// purpose" from "broken" without asking the cloud API, and reporting
		// `stopped` on a guess is precisely the thing that makes a remote node
		// feel broken — a wake button that appears for a machine nobody can
		// wake.
		// NO CREDENTIAL means no reading, so no opinion about the power either —
		// which is upUnknown and not upNo. Saying "it is off" here is the same
		// guess this branch exists to refuse.
		return s.setLocked(n.ID, StateUnreachable,
			"its provider is not on the bus, and the hub holds no cloud credentials for this node, "+
				"so it cannot tell a sleeping machine from a broken one", upUnknown)
	}
	st := s.st[n.ID]
	fs, err := cli.State(ctx, n.Fly.App, n.Fly.MachineID)
	if err != nil {
		// The cloud API did not answer. We learned nothing about the power, so
		// the previous belief stands rather than being overwritten by a guess.
		return s.setLocked(n.ID, StateUnreachable, "could not read the machine's state — "+describeFlyError(err), upUnknown)
	}
	// THE HELD EXIT RECORD IS NOW TWO RUNS OLD. lastExit is read off brain.info
	// while a node is UP and describes the run BEFORE the one answering; once
	// the machine is confirmed off, it no longer describes the ending that just
	// happened. Keeping it would show a crash notice from a previous life
	// beside a machine that has since been slept, which is the same class of
	// quiet wrongness this package exists to remove. Absent means NOBODY KNOWS,
	// and nobody does — until the next wake, when the node says so itself.
	if fs == flyapi.StateStopped || fs == flyapi.StateSuspended {
		st.lastExit = nil
	}
	switch fs {
	case flyapi.StateStarted:
		// RUNNING, AND PROVIDING NOTHING. This is the case the cloud API
		// cannot tell you about on its own and the one that costs money
		// silently: an entrypoint that dies, a brain that cannot dial out, a
		// token the node rejects. It is NOT `waking` — nobody asked it to
		// start — and it is certainly not `available`.
		return s.setLocked(n.ID, StateUnreachable,
			"the machine is running but its provider has not registered with the hub — "+
				"check the node's brain (--hub URL, token) and its boot log", upYes)
	case flyapi.StateStarting, flyapi.StateReplacing:
		return s.setLocked(n.ID, StateWaking, "the machine is starting", upYes)
	case flyapi.StateStopped, flyapi.StateSuspended:
		// THE ONE QUESTION THE CLOUD API CANNOT ANSWER, and where the sleep
		// path finally gives the hub a first-hand answer to it.
		//
		// `stopped` is what a machine somebody put to sleep looks like AND what
		// a machine looks like after the on-failure restart policy retried it
		// and gave up (`on-fail` on this side of the wire — flyapi records both
		// spellings). Byte-for-byte identical over the API, opposite meanings.
		// Three things tell them apart, in descending order of authority, and
		// each covers a case the others cannot:
		//
		//  1. THE HUB ISSUED THE STOP ITSELF (sleptByHub). First-hand, and the
		//     only one of the three readable while the machine is OFF — which
		//     is exactly when the question gets asked. In memory only, so a
		//     restarted hub falls through to 2 and 3, which is honest: it did
		//     not issue that stop and has no standing to claim it did.
		//  2. THE HUB'S OWN WAKE HISTORY (wakeFailures). Catches the crash
		//     LOOP: a node that never gets far enough to register.
		//  3. THE NODE'S OWN EXIT RECORD (lastExit). Catches the node that ran
		//     fine and then died — but it lives on the node's volume, so it
		//     only arrives on the NEXT successful wake. See lastexit.go.
		//
		// The order matters where 1 and 2 disagree: a node that failed to wake
		// and was then stopped BY THIS HUB to stop the bleeding has both bits
		// set, and it is not a healthy sleeper — the failure is the news, so
		// wakeFailures is checked first.
		if st.wakeFailures > 0 {
			slept := ""
			if st.sleptByHub {
				slept = " (the hub stopped it again so it would not keep billing)"
			}
			return s.setLocked(n.ID, StateUnreachable, fmt.Sprintf(
				"the machine is stopped, but %s did not end with its provider registering%s — "+
					"it may be failing on boot rather than sleeping",
				plural(st.wakeFailures, "the last wake attempt", "the last %d wake attempts"), slept), upNo)
		}
		if st.sleptByHub {
			return s.setLocked(n.ID, StateStopped, "this hub put it to sleep; nothing is billing", upNo)
		}
		// NOBODY KNOWS WHY THIS IS OFF, and that is a different sentence from
		// the one above rather than the same state with a blank detail. This
		// hub did not stop it and has no failed wakes to blame, so it is
		// either somebody else's deliberate stop or a crash whose account is
		// sitting on a volume that is not running. The node's own record
		// settles it one wake later.
		return s.setLocked(n.ID, StateStopped, "", upNo)
	case flyapi.StateDestroyed:
		return s.setLocked(n.ID, StateUnreachable, "the machine has been destroyed", upNo)
	default:
		// An unrecognised state string. It is surfaced rather than coerced, and
		// the power is left as UNKNOWN for the same reason: guessing either way
		// about a machine whose state we do not understand is how a client ends
		// up with a button that does nothing.
		return s.setLocked(n.ID, StateUnreachable, "the cloud API reports an unfamiliar state: "+oneLine(fs), upUnknown)
	}
}

func plural(n int, one, many string) string {
	if n == 1 {
		return one
	}
	return fmt.Sprintf(many, n)
}

// oneLine flattens a message so a detail string stays one sentence in a UI.
func oneLine(s string) string {
	s = strings.TrimSpace(strings.ReplaceAll(strings.ReplaceAll(s, "\n", " "), "\r", " "))
	const max = 240
	if len(s) > max {
		return s[:max] + "…"
	}
	return s
}

// describeFlyError renders a cloud-API failure for a CLIENT-FACING detail
// string, by category, and never by quoting the error.
//
// THIS IS THE SAME ALLOWLIST DIRECTION AS NodeView, applied to prose. The
// obvious thing — put the error text in the detail so the user can see it — is
// how the credential escapes: an error composed from a response BODY is
// composed by whatever answered the request, and a debug proxy that echoes the
// Authorization header back turns a helpful detail line into a token on
// somebody's phone screen. flyapi scrubs its own errors, and this is the
// second, independent barrier: the detail is built from things the hub knows
// (a status class, a category) rather than from bytes somebody else sent.
//
// It is pinned by TestNoPublishedChangeCarriesTheToken, which deliberately
// feeds in an error flyapi did NOT scrub.
func describeFlyError(err error) string {
	if err == nil {
		return "no reason given"
	}
	var rl *flyapi.RateLimitError
	if errors.As(err, &rl) {
		return "the cloud API is rate-limiting this machine; try again in a few seconds"
	}
	var apiErr *flyapi.APIError
	if errors.As(err, &apiErr) {
		switch {
		case apiErr.NotFound():
			return "the cloud API does not know this app or machine (check app and machineId in nodes.json)"
		case apiErr.Status == 401 || apiErr.Status == 403:
			return "the cloud API rejected this hub's credential (expired, revoked, or scoped to a different app)"
		case apiErr.Status >= 500:
			return fmt.Sprintf("the cloud API is failing (HTTP %d)", apiErr.Status)
		default:
			return fmt.Sprintf("the cloud API refused the request (HTTP %d)", apiErr.Status)
		}
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
		return "the cloud API did not answer in time"
	}
	return "the cloud API could not be reached"
}
