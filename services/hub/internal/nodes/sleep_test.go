package nodes

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/flyapi"
)

// fastSleep is the timing set every test in this file uses: real transitions,
// no real seconds. RegisterTimeout is deliberately tiny so a wake can be
// watched all the way to its failure without the suite waiting 90s for it.
func fastSleep() Tunables {
	return Tunables{
		PollInterval:         5 * time.Millisecond,
		ProbeTimeout:         50 * time.Millisecond,
		RegisterTimeout:      60 * time.Millisecond,
		RegisterPollInterval: 5 * time.Millisecond,
		StopGrace:            20 * time.Millisecond,
		StopTimeout:          40 * time.Millisecond,
		StartRetryDelay:      time.Millisecond,
	}
}

func (h *harness) waitForStops(t *testing.T, want int) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for h.fly.stopCount() < want {
		if time.Now().After(deadline) {
			t.Fatalf("the hub issued %d stop(s), want %d", h.fly.stopCount(), want)
		}
		time.Sleep(2 * time.Millisecond)
	}
}

// ── the sleep verb ─────────────────────────────────────────────────────────

// THE SIGNAL AND THE DRAIN WINDOW MUST BE ON THE WIRE, AND THEY MUST BE THE
// SUPERVISOR'S.
//
// fly.toml's kill_signal / kill_timeout govern a PLATFORM stop; an API stop
// never reads them. A stop that leaves either to the cloud API's default
// SIGKILLs a node mid-flush — and a SIGKILLed node writes no exit record, which
// is the one artefact that tells the next wake this stop was deliberate.
func TestSleepStopsTheMachineWithAnExplicitSignalAndDrainWindow(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())
	h.waitForState(t, "den", StateAvailable)

	v, err := h.sup.Sleep(context.Background(), "den")
	if err != nil {
		t.Fatalf("Sleep: %v", err)
	}
	if v.State != string(StateStopping) {
		t.Errorf("Sleep returned state %q, want stopping — it must return before the machine is off, exactly as Wake returns before it is on", v.State)
	}
	h.waitForStops(t, 1)

	sigs, timeouts := h.fly.stopArgs()
	if len(sigs) != 1 {
		t.Fatalf("stop signals recorded = %v", sigs)
	}
	if sigs[0] != "SIGTERM" {
		t.Errorf("the hub stopped with signal %q, want SIGTERM — the node's entrypoint traps INT and TERM and writes its exit record on either; a SIGKILL leaves no record and the next wake cannot tell this from a crash", sigs[0])
	}
	if timeouts[0] <= 0 {
		t.Errorf("the hub stopped with a drain timeout of %s — fly.toml's kill_timeout does not govern an API stop, so a zero here is the cloud API's default and not the deployment's", timeouts[0])
	}
	h.waitForState(t, "den", StateStopped)
}

// The node's own exit record is written on a signal the entrypoint can trap.
// This pins the pair rather than the constant: the signal the supervisor sends
// must be one ExitRecord.Clean() will recognise when it comes back one wake
// later, or the two halves of the crash-vs-sleep answer disagree.
func TestTheStopSignalIsOneTheNodesExitRecordWillCallClean(t *testing.T) {
	tun := Tunables{}.withDefaults()
	sig := strings.TrimPrefix(tun.StopSignal, "SIG")
	rec := &ExitRecord{Reason: "signal-" + sig}
	if !rec.Clean() {
		t.Fatalf("the supervisor stops with %q, and a node that records that ending (%q) is NOT read back as a clean stop — the hub would call its own deliberate sleep a crash on the next wake", tun.StopSignal, rec.Reason)
	}
}

// Three clients tapping Sleep produce ONE stop call. The cloud API allows one
// action per second per machine, and a second signal into a draining process
// tree is how a clean flush becomes a half-written one.
func TestSleepIsIdempotentPerNode(t *testing.T) {
	tun := fastSleep()
	tun.StopTimeout = 2 * time.Second
	h := newHarness(t, []Node{flyNode("den")}, tun)
	h.fly.setState(flyapi.StateStarted)
	// Hold the stop open so all three calls land while the first is in flight.
	release := make(chan struct{})
	h.fly.onStop = func() { <-release }

	done := make(chan struct{}, 3)
	for i := 0; i < 3; i++ {
		go func() { _, _ = h.sup.Sleep(context.Background(), "den"); done <- struct{}{} }()
	}
	time.Sleep(60 * time.Millisecond)
	close(release)
	for i := 0; i < 3; i++ {
		<-done
	}
	if n := h.fly.stopCount(); n != 1 {
		t.Errorf("three concurrent sleeps issued %d stop calls, want 1 — the cloud API allows one action per second per machine and three is a 429", n)
	}
}

// Sleeping a node that is already off spends nothing at all.
func TestSleepingAnAlreadyStoppedNodeSpendsNothing(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStopped)
	h.sup.Reconcile(context.Background())
	h.waitForState(t, "den", StateStopped)
	if _, err := h.sup.Sleep(context.Background(), "den"); err != nil {
		t.Fatalf("Sleep on a stopped node: %v", err)
	}
	if n := h.fly.stopCount(); n != 0 {
		t.Errorf("sleeping an already-stopped node issued %d stop call(s)", n)
	}
}

// A node the hub holds no credential for gets a refusal that says so — not a
// stop that fails, and not a button that was offered in the first place.
func TestANodeWithNoCredentialCannotBeSlept(t *testing.T) {
	h := newHarness(t, []Node{{ID: "laptop", Label: "someone's laptop"}}, fastSleep())
	_, err := h.sup.Sleep(context.Background(), "laptop")
	if !errors.Is(err, ErrNotSleepable) {
		t.Fatalf("Sleep on a credential-less node = %v, want ErrNotSleepable", err)
	}
	if _, err := h.sup.Sleep(context.Background(), "nope"); !errors.Is(err, ErrUnknownNode) {
		t.Errorf("Sleep on an unregistered id = %v, want ErrUnknownNode", err)
	}
}

// A stop that is accepted and never completes is the EXPENSIVE case: the
// machine is very likely still running. It must not land in `stopped`.
func TestASleepThatNeverCompletesSaysTheMachineMayStillBeBilling(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	h.fly.stopWaitErr = errors.New("timed out waiting for stopped")
	if _, err := h.sup.Sleep(context.Background(), "den"); err != nil {
		t.Fatalf("Sleep: %v", err)
	}
	v := h.waitForState(t, "den", StateUnreachable)
	if !strings.Contains(v.Detail, "billing") {
		t.Errorf("a sleep that never completed says %q — it must name the bill, because a machine that did not stop is a machine that is still being paid for", v.Detail)
	}
	if v.SleptByHub {
		t.Error("a sleep that never completed claims the hub put the machine to sleep; it did not — the machine may still be running")
	}
}

// A cloud API that refuses the stop outright is the same class of answer and
// gets the same honesty, plus the error itself back to the caller.
func TestASleepTheCloudAPIRefusesIsReportedToTheCaller(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	h.fly.stopErr = &flyapi.APIError{Status: 403, Body: "nope"}
	_, err := h.sup.Sleep(context.Background(), "den")
	if err == nil {
		t.Fatal("a refused stop returned no error")
	}
	v := h.view(t, "den")
	if v.State != string(StateUnreachable) {
		t.Errorf("state after a refused stop = %q, want unreachable", v.State)
	}
	if !strings.Contains(v.Detail, "billing") {
		t.Errorf("a refused stop says %q and never mentions that the machine is still running", v.Detail)
	}
}

// ── the failed wake, which is what this whole path exists to close ──────────

// THE HOLE THE WAKE-ONLY V1 SHIPPED WITH.
//
// A wake started a billable machine, the machine never became usable, and
// nothing in the app could turn it off again — with no public IP there is not
// even ambient traffic to notice it. It billed until somebody opened the cloud
// console. This is the test that says it does not any more.
func TestAFailedWakeStopsTheMachineItStarted(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStopped)
	h.bus.set("", ErrNoProvider, false) // the node never registers

	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	h.waitForStops(t, 1)

	sigs, timeouts := h.fly.stopArgs()
	if sigs[0] == "" || timeouts[0] <= 0 {
		t.Errorf("the failed-wake stop went out with signal=%q timeout=%s — every stop this hub issues names both explicitly", sigs[0], timeouts[0])
	}
	v := h.waitForState(t, "den", StateUnreachable)
	if !strings.Contains(v.Detail, "STOPPED IT AGAIN") {
		t.Errorf("detail after a failed wake = %q; it must say the machine was stopped again, because the previous version of this sentence said it 'may still be running' and that was the known cost", v.Detail)
	}
	if v.WakeFailures != 1 {
		t.Errorf("wakeFailures = %d, want 1 — stopping the machine does not make the failed wake unhappen", v.WakeFailures)
	}
	if v.SleptByHub {
		t.Error("a machine stopped because it failed to boot must NOT be marked as one the hub put to sleep — that would render a node that keeps dying as a healthy sleeper")
	}
}

// A machine stopped after a failed wake reconciles as UNREACHABLE, not as a
// healthy sleeper. Through the cloud API those are the same string; what tells
// them apart is the hub's own wake history.
func TestAMachineStoppedAfterAFailedWakeIsNotAHealthySleeper(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStopped)
	h.bus.set("", ErrNoProvider, false)
	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	h.waitForStops(t, 1)
	h.waitForState(t, "den", StateUnreachable)

	h.sup.Reconcile(context.Background())
	v := h.view(t, "den")
	if v.State != string(StateUnreachable) {
		t.Fatalf("a machine that failed to boot and was stopped reconciles as %q — the cloud API says `stopped` for that AND for a healthy sleeper, and calling it stopped is the quiet wrongness this package exists to remove", v.State)
	}
	if !strings.Contains(v.Detail, "failing on boot") {
		t.Errorf("detail = %q; it must say the machine may be failing on boot", v.Detail)
	}
}

// The escape hatch: leave a broken boot up so someone can look at it. Off by
// default, because the zero value has to be the one that does not bill.
func TestKeepFailedWakesRunningLeavesTheMachineUp(t *testing.T) {
	tun := fastSleep()
	tun.KeepFailedWakesRunning = true
	h := newHarness(t, []Node{flyNode("den")}, tun)
	h.fly.setState(flyapi.StateStopped)
	h.bus.set("", ErrNoProvider, false)
	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	h.waitForState(t, "den", StateUnreachable)
	time.Sleep(80 * time.Millisecond)
	if n := h.fly.stopCount(); n != 0 {
		t.Errorf("KeepFailedWakesRunning is set and the hub still issued %d stop(s)", n)
	}
	// The zero value must be the safe one: an operator who never heard of this
	// field gets the machine stopped.
	if (Tunables{}).withDefaults().KeepFailedWakesRunning {
		t.Error("KeepFailedWakesRunning defaults to true — the zero value of a flag that leaves machines billing must be the one that does not")
	}
}

// A start the cloud API REFUSED started nothing, so there is nothing to stop
// and no bill to close. Issuing one anyway would be the hub acting on a machine
// it has no reason to believe is running.
func TestNoStopIsIssuedWhenTheStartItselfFailed(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.startErr = &flyapi.APIError{Status: 404, Body: "no such machine"}
	if _, err := h.sup.Wake(context.Background(), "den"); err == nil {
		t.Fatal("expected the refused start to be returned")
	}
	time.Sleep(80 * time.Millisecond)
	if n := h.fly.stopCount(); n != 0 {
		t.Errorf("a start the cloud API refused was followed by %d stop call(s) — nothing was started, so nothing may be stopped", n)
	}
}

// ── THE MUTATION GUARD ─────────────────────────────────────────────────────

// NOTHING AUTOMATIC IN THIS HUB MAY SWITCH OFF A MACHINE SOMEBODY IS USING.
//
// Adding a stop verb is a capability addition, not a refactor, and this is the
// invariant that confines the ONE stop no human asked for. Break either half of
// mayAutoStopLocked — drop the `state != StateAvailable` clause, or drop the
// generation check — and this test fails by name.
//
// Both halves are exercised directly rather than only end-to-end, because the
// end-to-end claim ("failWake is unreachable from an available node") is a
// statement about today's call graph, and the thing worth pinning is a
// statement about the machine.
func TestTheHubNeverStopsANodeWhoseProviderIsAnswering(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())
	h.waitForState(t, "den", StateAvailable)

	// The guard itself, at the generation the node is actually on.
	h.sup.mu.Lock()
	gen := h.sup.st["den"].gen
	mayAvailable := h.sup.mayAutoStopLocked("den", gen)
	h.sup.mu.Unlock()
	if mayAvailable {
		t.Error("the automatic stop would fire against an AVAILABLE node — a node whose provider is answering is one somebody may be typing at, and no automatic path in this hub may switch that off")
	}

	// The generation half: a stale watcher must not act on a node that has
	// since been asked to do something else. This is the window a user opens by
	// pressing Connect again on a node that is taking too long.
	h.sup.mu.Lock()
	h.sup.st["den"].state = StateUnreachable
	h.sup.st["den"].gen++
	mayStale := h.sup.mayAutoStopLocked("den", gen)
	h.sup.mu.Unlock()
	if mayStale {
		t.Error("the automatic stop would fire on a STALE generation — a wake that landed while the previous wake's watcher was waiting would have its machine stopped out from under it")
	}

	// And end to end: a wake that succeeds never stops anything, however long
	// the register window is left to elapse afterwards.
	h2 := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h2.fly.setState(flyapi.StateStopped)
	h2.bus.set("den", nil, true)
	if _, err := h2.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	h2.waitForState(t, "den", StateAvailable)
	time.Sleep(120 * time.Millisecond) // well past RegisterTimeout
	if n := h2.fly.stopCount(); n != 0 {
		t.Errorf("a wake that SUCCEEDED was followed by %d stop call(s)", n)
	}
}

// A sleep pressed mid-wake abandons the wake rather than racing it. Without the
// generation counter, the wake's watcher would report `available` for a machine
// that is draining and a composer would open at it.
func TestSleepAbandonsAWakeInFlight(t *testing.T) {
	tun := fastSleep()
	tun.RegisterTimeout = 2 * time.Second
	h := newHarness(t, []Node{flyNode("den")}, tun)
	h.fly.setState(flyapi.StateStopped)
	h.bus.set("", ErrNoProvider, false)
	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	h.waitForState(t, "den", StateWaking)

	// The node's provider comes up JUST as the user gives up on it.
	h.bus.set("den", nil, true)
	if _, err := h.sup.Sleep(context.Background(), "den"); err != nil {
		t.Fatalf("Sleep: %v", err)
	}
	h.waitForState(t, "den", StateStopped)

	time.Sleep(60 * time.Millisecond)
	if v := h.view(t, "den"); v.State != string(StateStopped) {
		t.Errorf("the abandoned wake's watcher moved the node to %q after the sleep completed — a stale watcher must never write", v.State)
	}
}

// ── reconciliation: telling a deliberate sleep from a crash ─────────────────

// THE QUESTION NO CLOUD API CAN ANSWER. `stopped` is what a machine somebody
// put to sleep looks like AND what one looks like after the on-failure restart
// policy gave up. This is the hub's own first-hand answer, and the only one
// readable while the node is OFF.
func TestADeliberateSleepIsDistinguishableFromACrashedMachine(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())
	h.waitForState(t, "den", StateAvailable)

	h.bus.set("", ErrNoProvider, false)
	if _, err := h.sup.Sleep(context.Background(), "den"); err != nil {
		t.Fatalf("Sleep: %v", err)
	}
	h.waitForState(t, "den", StateStopped)

	h.sup.Reconcile(context.Background())
	slept := h.view(t, "den")
	if !slept.SleptByHub {
		t.Fatal("a machine this hub stopped does not say so — that bit is the only account of the stop available while the machine is off, because the node's own record lives on a volume that is not running")
	}
	if slept.Detail == "" {
		t.Error("a deliberately slept node renders no sentence at all; it must be distinguishable on screen from one that is merely stopped")
	}

	// A machine nobody here stopped: same cloud state, no claim.
	h2 := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h2.fly.setState(flyapi.StateStopped)
	h2.sup.Reconcile(context.Background())
	other := h2.view(t, "den")
	if other.State != string(StateStopped) {
		t.Fatalf("state = %q, want stopped", other.State)
	}
	if other.SleptByHub {
		t.Error("a hub claims to have slept a machine it never touched — absent must mean 'this hub did not do it', never 'somebody else did'")
	}
}

// The claim does not survive a hub restart, and that is honest rather than a
// gap. A hub that has just booted did not issue that stop.
func TestTheSleptByHubClaimDoesNotSurviveARestart(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	if _, err := h.sup.Sleep(context.Background(), "den"); err != nil {
		t.Fatalf("Sleep: %v", err)
	}
	h.waitForState(t, "den", StateStopped)

	// A "restarted" hub is a new supervisor over the same registry — the hub
	// keeps no node state on disk, so this IS the restart path.
	fresh := New(Options{Nodes: []Node{flyNode("den")}, Bus: h.bus, Clients: map[string]flyapi.Client{"den": h.fly}})
	fresh.Reconcile(context.Background())
	v, err := fresh.View("den")
	if err != nil {
		t.Fatalf("View: %v", err)
	}
	if v.SleptByHub {
		t.Error("a freshly booted hub claims it put this machine to sleep; it did not, and there is no file it could honestly have learned that from")
	}
}

// A wake clears the claim: whatever the last stop was, it is no longer the
// answer to "why is this off".
func TestWakingClearsTheSleptByHubClaim(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	if _, err := h.sup.Sleep(context.Background(), "den"); err != nil {
		t.Fatalf("Sleep: %v", err)
	}
	h.waitForState(t, "den", StateStopped)
	h.bus.set("den", nil, true)
	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	h.waitForState(t, "den", StateAvailable)
	if h.view(t, "den").SleptByHub {
		t.Error("a woken node still claims the hub has it asleep")
	}
}

// THE STALE EXIT RECORD. lastExit is read while a node is UP and describes the
// run BEFORE the one answering. Once the machine is off, it is two runs old —
// showing it beside a stopped machine reports a crash from a previous life as
// though it were news. Absent means NOBODY KNOWS, and nobody does.
func TestAStoppedMachineDropsTheExitRecordTheHubWasHolding(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, fastSleep())
	h.fly.setState(flyapi.StateStarted)
	h.bus.setExit(&ExitRecord{Reason: "claudemon-died", At: "2026-08-24T21:00:00Z"})
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())
	if h.view(t, "den").LastExit == nil {
		t.Fatal("the hub did not pick up the node's exit record at all")
	}

	h.bus.set("", ErrNoProvider, false)
	h.bus.setExit(nil)
	h.fly.setState(flyapi.StateStopped)
	h.sup.Reconcile(context.Background())

	v := h.view(t, "den")
	if v.LastExit != nil {
		t.Errorf("a machine that is now OFF still carries the exit record of a run two generations back (%+v) — that record does not describe the ending that just happened, and a missing one honestly means nobody knows", v.LastExit)
	}
}

// ── mayBeRunning: is there a meter to switch off ───────────────────────────

// THE QUESTION A STOP BUTTON HAS TO ASK, and the one `state` cannot answer.
//
// `unreachable` covers a machine that is RUNNING and providing nothing — a
// meter, and the reason the button exists — AND a machine that is off and
// broken, where the same button would do nothing. The hub knows which; this is
// where it says so.
//
// THE REGRESSION THIS FIXES BY CONSTRUCTION: the first version of the client
// sniffed the hub's `detail` for the word "billing". The detail for a machine
// the hub had ALREADY stopped reads "…so it would not keep billing", so the
// regex called an off machine a running one and offered a dead button. The last
// case below is exactly that sentence.
func TestMayBeRunningSeparatesAMeterFromABrokenOffMachine(t *testing.T) {
	t.Run("running and providing nothing is a meter", func(t *testing.T) {
		h := newHarness(t, []Node{flyNode("den")}, fastSleep())
		h.fly.setState(flyapi.StateStarted)
		h.sup.Reconcile(context.Background())
		v := h.view(t, "den")
		if v.State != string(StateUnreachable) {
			t.Fatalf("state = %q, want unreachable", v.State)
		}
		if !v.MayBeRunning {
			t.Error("a machine the cloud API reports as `started` is not marked as possibly running — a client cannot tell this from an off-and-broken node, and the off switch is the whole point")
		}
	})

	t.Run("stopped is not a meter", func(t *testing.T) {
		h := newHarness(t, []Node{flyNode("den")}, fastSleep())
		h.fly.setState(flyapi.StateStopped)
		h.sup.Reconcile(context.Background())
		if h.view(t, "den").MayBeRunning {
			t.Error("a stopped machine is marked as possibly running")
		}
	})

	t.Run("a machine the hub could not read keeps the previous belief", func(t *testing.T) {
		// upUnknown: the cloud API did not answer, so we learned NOTHING about
		// the power. Overwriting the belief with a guess is how a live machine
		// loses its off switch during a blip.
		h := newHarness(t, []Node{flyNode("den")}, fastSleep())
		h.fly.setState(flyapi.StateStarted)
		h.sup.Reconcile(context.Background())
		if !h.view(t, "den").MayBeRunning {
			t.Fatal("setup: expected the machine to read as running")
		}
		h.fly.stateErr = errors.New("the cloud API is unreachable")
		h.sup.Reconcile(context.Background())
		if !h.view(t, "den").MayBeRunning {
			t.Error("a failed READING cleared the belief that the machine is up — the hub learned nothing and must not guess it off")
		}
	})

	t.Run("THE REGEX BUG: a stopped machine whose detail says billing", func(t *testing.T) {
		h := newHarness(t, []Node{flyNode("den")}, fastSleep())
		h.fly.setState(flyapi.StateStopped)
		h.bus.set("", ErrNoProvider, false)
		if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
			t.Fatalf("Wake: %v", err)
		}
		h.waitForStops(t, 1)
		h.waitForState(t, "den", StateUnreachable)

		// Read it HERE, before the next reconcile rewrites the sentence: this is
		// the row a client is actually holding in the moment after a failed wake
		// is cleaned up, and it is the one the regex read backwards.
		v := h.view(t, "den")
		if !strings.Contains(v.Detail, "billing") {
			t.Fatalf("the detail after the hub stopped a failed wake no longer contains the word this case exists for: %q. Re-derive the case rather than deleting it — the point is that a sentence about NOT billing describes a machine that is OFF.", v.Detail)
		}
		if v.MayBeRunning {
			t.Fatalf("a machine THE HUB ITSELF STOPPED is marked as possibly running because its sentence contains the word \"billing\": %q. This is the exact reading a prose sniff got wrong, and it is why the answer is a field.", v.Detail)
		}
	})
}

// ── the token, over the new surface ────────────────────────────────────────

// The credential must not escape through anything the sleep path added: not the
// view, not the detail sentences, not the published change, and not the error a
// refused stop hands back to the caller.
//
// The stub error is one flyapi did NOT scrub — the shape a debug proxy that
// echoes request headers actually produces — so this tests the SECOND barrier
// (describeFlyError renders by category, never by quoting) independently of the
// first.
func TestNoSleepPathEverRendersTheFlyToken(t *testing.T) {
	h := newHarness(t, []Node{nodeWithSecret()}, fastSleep())
	h.sup.clients[nodeWithSecret().ID] = h.fly
	h.fly.setState(flyapi.StateStarted)
	h.fly.stopErr = errors.New("fly: HTTP 502: {\"headers\":{\"authorization\":\"Bearer " + secretToken + "\"}}")

	var published []Change
	h.sup.publish = func(c Change) { published = append(published, c) }

	_, err := h.sup.Sleep(context.Background(), nodeWithSecret().ID)
	if err == nil {
		t.Fatal("expected the refused stop to be returned")
	}
	// The raw error the caller gets IS the cloud client's, and this fake did
	// not scrub it — what must not happen is that text reaching a NodeView, a
	// detail sentence or a published event.
	for _, c := range published {
		raw, _ := json.Marshal(c.View)
		if strings.Contains(string(raw), secretToken) {
			t.Fatalf("A PUBLISHED node.state_changed CARRIED THE FLY TOKEN: %s", raw)
		}
	}
	raw, _ := json.Marshal(h.view(t, nodeWithSecret().ID))
	if strings.Contains(string(raw), secretToken) {
		t.Fatalf("THE FLY TOKEN IS IN THE CLIENT-FACING VIEW AFTER A FAILED SLEEP: %s", raw)
	}
	for _, leak := range []string{"wks-node-den", "17811944b12345", "api.machines.dev"} {
		if strings.Contains(string(raw), leak) {
			t.Errorf("the sleep path put the Fly identifier %q on the wire: %s", leak, raw)
		}
	}
}

// ── the state itself ───────────────────────────────────────────────────────

func TestStoppingIsAValidStateAndIsNeitherStoppedNorUnreachable(t *testing.T) {
	if !StateStopping.Valid() {
		t.Fatal("stopping is not a valid state")
	}
	for _, other := range []State{StateAvailable, StateWaking, StateStopped, StateUnreachable} {
		if StateStopping == other {
			t.Fatalf("stopping collapsed into %q", other)
		}
	}
}
