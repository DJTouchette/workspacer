package nodes

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/flyapi"
)

// ---- fakes ---------------------------------------------------------------

// fakeBus stands in for the hub bus. Nothing in this package's tests opens a
// socket, and nothing reaches a real cloud API.
type fakeBus struct {
	mu sync.Mutex
	// answer: "" + nil err  → a brain answered but named no node
	//         id + nil err  → a brain answered naming that node
	//         ErrNoProvider → nothing registered
	//         other error   → registered but silent (the zombie)
	answer     string
	lastExit   *ExitRecord
	err        error
	registered bool
	evictions  int
	probes     int
}

func (f *fakeBus) ProbeBrain(context.Context) (Probe, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.probes++
	return Probe{NodeID: f.answer, LastExit: f.lastExit}, f.err
}
func (f *fakeBus) BrainProviderRegistered() bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.registered
}
func (f *fakeBus) EvictBrainProvider() bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	if !f.registered {
		return false
	}
	f.registered = false
	f.evictions++
	f.err = ErrNoProvider
	f.answer = ""
	return true
}
func (f *fakeBus) setExit(rec *ExitRecord) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.lastExit = rec
}
func (f *fakeBus) set(answer string, err error, registered bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.answer, f.err, f.registered = answer, err, registered
}
func (f *fakeBus) evictionCount() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.evictions
}

// fakeFly is a Fly Machines API that costs nothing.
type fakeFly struct {
	mu       sync.Mutex
	state    string
	starts   int
	startErr error
	waitErr  error
	stateErr error
	onStart  func()
	// stopWaitErr, when set, is what WaitForState returns for a stop wait
	// specifically — a wake wait and a sleep wait fail for different reasons
	// and the tests need to drive them apart.
	stopWaitErr error
	waitCalled  int

	// The stop half. `stopSignals` and `stopTimeouts` record what the caller
	// actually put on the wire, because "it called Stop" and "it called Stop
	// with a drain window the deployment chose" are different claims.
	stops        int
	stopErr      error
	stopSignals  []string
	stopTimeouts []time.Duration
	onStop       func()
}

func (f *fakeFly) Start(context.Context, string, string) error {
	f.mu.Lock()
	f.starts++
	err, hook := f.startErr, f.onStart
	if err == nil {
		f.state = flyapi.StateStarted
	}
	f.mu.Unlock()
	if hook != nil {
		hook()
	}
	return err
}
func (f *fakeFly) Stop(_ context.Context, _, _, signal string, timeout time.Duration) error {
	f.mu.Lock()
	f.stops++
	f.stopSignals = append(f.stopSignals, signal)
	f.stopTimeouts = append(f.stopTimeouts, timeout)
	err, hook := f.stopErr, f.onStop
	if err == nil {
		f.state = flyapi.StateStopped
	}
	f.mu.Unlock()
	if hook != nil {
		hook()
	}
	return err
}

func (f *fakeFly) State(context.Context, string, string) (string, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.state, f.stateErr
}
func (f *fakeFly) WaitForState(_ context.Context, _, _, want string, _ time.Duration) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.waitCalled++
	if want == flyapi.StateStopped {
		return f.stopWaitErr
	}
	return f.waitErr
}
func (f *fakeFly) startCount() int   { f.mu.Lock(); defer f.mu.Unlock(); return f.starts }
func (f *fakeFly) stopCount() int    { f.mu.Lock(); defer f.mu.Unlock(); return f.stops }
func (f *fakeFly) setState(s string) { f.mu.Lock(); f.state = s; f.mu.Unlock() }
func (f *fakeFly) stopArgs() ([]string, []time.Duration) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string(nil), f.stopSignals...), append([]time.Duration(nil), f.stopTimeouts...)
}

func flyNode(id string) Node {
	return Node{ID: id, Label: id + " label", Fly: &Fly{App: "app-" + id, MachineID: "m-" + id, Token: secretToken}}
}

type harness struct {
	sup *Supervisor
	bus *fakeBus
	fly *fakeFly
	ch  chan Change
}

func newHarness(t *testing.T, nodes []Node, tun Tunables) *harness {
	t.Helper()
	h := &harness{bus: &fakeBus{err: ErrNoProvider}, fly: &fakeFly{state: flyapi.StateStopped}, ch: make(chan Change, 64)}
	clients := map[string]flyapi.Client{}
	for _, n := range nodes {
		if n.Wakeable() {
			clients[n.ID] = h.fly
		}
	}
	h.sup = New(Options{
		Nodes: nodes, Bus: h.bus, Clients: clients, Tunables: tun,
		Logf: func(string, ...any) {},
		Publish: func(c Change) {
			select {
			case h.ch <- c:
			default:
			}
		},
	})
	return h
}

func (h *harness) view(t *testing.T, id string) NodeView {
	t.Helper()
	v, err := h.sup.View(id)
	if err != nil {
		t.Fatalf("View(%q): %v", id, err)
	}
	return v
}

func (h *harness) waitForState(t *testing.T, id string, want State) NodeView {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for {
		v := h.view(t, id)
		if v.State == string(want) {
			return v
		}
		if time.Now().After(deadline) {
			t.Fatalf("node %q stayed %q, want %q (detail: %s)", id, v.State, want, v.Detail)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// ---- the state machine ---------------------------------------------------

// A hub that has just started believes nothing, and says so rather than
// guessing. This is the "never trust in-memory state across a restart" rule
// made structural: there IS no state to trust.
func TestABootedHubBelievesNothingUntilItReconciles(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	v := h.view(t, "den")
	if v.State != string(StateUnreachable) {
		t.Errorf("state before reconcile = %q, want unreachable", v.State)
	}
	if v.Detail == "" {
		t.Error("a node in a state the hub has not verified must say why")
	}
}

func TestReconcileDerivesEveryState(t *testing.T) {
	cases := []struct {
		name       string
		flyState   string
		probeID    string
		probeErr   error
		registered bool
		want       State
	}{
		{"provider answering is available", flyapi.StateStarted, "den", nil, true, StateAvailable},
		{"stopped machine with no provider is stopped", flyapi.StateStopped, "", ErrNoProvider, false, StateStopped},
		{"suspended machine is stopped", flyapi.StateSuspended, "", ErrNoProvider, false, StateStopped},
		{"machine starting is waking", flyapi.StateStarting, "", ErrNoProvider, false, StateWaking},
		// THE ONE THE CLOUD API CANNOT TELL YOU: running, and providing
		// nothing. Not stopped, not available, and certainly not fine.
		{"running with no provider is unreachable", flyapi.StateStarted, "", ErrNoProvider, false, StateUnreachable},
		{"destroyed is unreachable", flyapi.StateDestroyed, "", ErrNoProvider, false, StateUnreachable},
		{"an unfamiliar state is unreachable", "teleporting", "", ErrNoProvider, false, StateUnreachable},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			h := newHarness(t, []Node{flyNode("den")}, Tunables{})
			h.fly.setState(tc.flyState)
			h.bus.set(tc.probeID, tc.probeErr, tc.registered)
			h.sup.Reconcile(context.Background())
			if got := h.view(t, "den").State; got != string(tc.want) {
				t.Errorf("state = %q, want %q (detail: %s)", got, tc.want, h.view(t, "den").Detail)
			}
		})
	}
}

// A node with no credential is never called `stopped`, because the hub has no
// way to know that and a wake button on it would fail every time.
func TestANodeWithNoCredentialIsUnreachableNotStopped(t *testing.T) {
	h := newHarness(t, []Node{{ID: "laptop", Label: "The laptop"}}, Tunables{})
	h.sup.Reconcile(context.Background())
	v := h.view(t, "laptop")
	if v.State != string(StateUnreachable) {
		t.Errorf("state = %q, want unreachable", v.State)
	}
	if v.Wakeable {
		t.Error("a node with no credential must not advertise itself as wakeable")
	}
}

// Coordinates without a credential is the same answer: the hub cannot act.
func TestCoordinatesWithoutACredentialAreNotWakeable(t *testing.T) {
	n := flyNode("den")
	sup := New(Options{Nodes: []Node{n}, Bus: &fakeBus{err: ErrNoProvider}, Clients: map[string]flyapi.Client{}})
	v, _ := sup.View("den")
	if v.Wakeable {
		t.Error("wakeable = true with no Fly client registered for the node")
	}
	if _, err := sup.Wake(context.Background(), "den"); !errors.Is(err, ErrNotWakeable) {
		t.Errorf("Wake error = %v, want ErrNotWakeable", err)
	}
}

// THE CRASH-LOOP CASE. A machine whose entrypoint dies exhausts its restart
// policy and ends up `stopped` — indistinguishable through the cloud API from
// a healthy sleeping node. The hub's own wake history is the only thing that
// can tell them apart, so it must.
func TestAStoppedMachineAfterAFailedWakeIsUnreachableNotStopped(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{RegisterTimeout: 60 * time.Millisecond, ProbeTimeout: 20 * time.Millisecond})
	h.bus.set("", ErrNoProvider, false)

	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	h.waitForState(t, "den", StateUnreachable) // the wake timed out

	// The machine gave up and went back to stopped.
	h.fly.setState(flyapi.StateStopped)
	h.sup.Reconcile(context.Background())
	v := h.view(t, "den")
	if v.State != string(StateUnreachable) {
		t.Fatalf("state = %q, want unreachable — a stopped machine after a failed wake is NOT 'asleep and fine'", v.State)
	}
	if v.WakeFailures == 0 {
		t.Error("wakeFailures = 0 after a wake that never produced a registration")
	}
	if v.Detail == "" {
		t.Error("the hub must say why this is not simply 'stopped'")
	}
}

// ---- wake ----------------------------------------------------------------

func TestWakeStartsTheMachineAndReportsWakingImmediately(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{ProbeTimeout: 20 * time.Millisecond, RegisterPollInterval: 5 * time.Millisecond})
	// The machine boots and its brain registers shortly after the start call.
	h.fly.onStart = func() { h.bus.set("den", nil, true) }

	v, err := h.sup.Wake(context.Background(), "den")
	if err != nil {
		t.Fatalf("Wake: %v", err)
	}
	if v.State != string(StateWaking) {
		t.Fatalf("Wake returned state %q, want waking — the whole point is that the caller does not block", v.State)
	}
	h.waitForState(t, "den", StateAvailable)
	if h.fly.startCount() != 1 {
		t.Errorf("start calls = %d, want 1", h.fly.startCount())
	}
}

// Three clients tapping Wake produce ONE machine start. Not tidiness: the
// cloud API allows one action per second per machine, and a 429 on the wake
// path reads to a user as "the button does nothing".
func TestConcurrentWakesProduceExactlyOneStart(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{ProbeTimeout: 20 * time.Millisecond, RegisterTimeout: 2 * time.Second, RegisterPollInterval: 5 * time.Millisecond})
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() { defer wg.Done(); _, _ = h.sup.Wake(context.Background(), "den") }()
	}
	wg.Wait()
	if got := h.fly.startCount(); got != 1 {
		t.Errorf("start calls = %d, want exactly 1", got)
	}
	h.bus.set("den", nil, true)
	h.waitForState(t, "den", StateAvailable)
}

// Waking a node that is already up spends nothing.
func TestWakingAnAvailableNodeCallsNoCloudAPI(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	h.bus.set("den", nil, true)
	h.fly.setState(flyapi.StateStarted)
	h.sup.Reconcile(context.Background())

	v, err := h.sup.Wake(context.Background(), "den")
	if err != nil {
		t.Fatalf("Wake: %v", err)
	}
	if v.State != string(StateAvailable) {
		t.Errorf("state = %q, want available", v.State)
	}
	if h.fly.startCount() != 0 {
		t.Errorf("start calls = %d, want 0 — waking a running node must not spend money", h.fly.startCount())
	}
}

func TestWakeOnAnUnknownNodeIsAnError(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	if _, err := h.sup.Wake(context.Background(), "nope"); !errors.Is(err, ErrUnknownNode) {
		t.Errorf("Wake error = %v, want ErrUnknownNode", err)
	}
}

// A 404 from the cloud API means the coordinates are wrong; asking again would
// be wrong again, so it is not retried and the node lands unreachable.
func TestAStartThatIsRefusedLandsUnreachableAndCountsAsAFailure(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	h.fly.startErr = &flyapi.APIError{Status: 404, Body: `{"error":"machine not found"}`}
	if _, err := h.sup.Wake(context.Background(), "den"); err == nil {
		t.Fatal("expected Wake to report the refusal")
	}
	if h.fly.startCount() != 1 {
		t.Errorf("start calls = %d, want 1 — a 404 must not be retried", h.fly.startCount())
	}
	v := h.view(t, "den")
	if v.State != string(StateUnreachable) || v.WakeFailures != 1 {
		t.Errorf("after a refused start: state=%q failures=%d, want unreachable/1", v.State, v.WakeFailures)
	}
}

// A transient failure IS retried, once.
func TestATransientStartFailureIsRetriedOnce(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{StartRetryDelay: time.Millisecond})
	h.fly.startErr = &flyapi.APIError{Status: 502, Body: "bad gateway"}
	_, _ = h.sup.Wake(context.Background(), "den")
	if got := h.fly.startCount(); got != 2 {
		t.Errorf("start calls = %d, want 2 (one retry)", got)
	}
}

// A wake that starts a machine but never gets a provider must end honestly —
// and must NOT leave the node parked in `waking` forever.
func TestAWakeThatNeverRegistersEndsUnreachable(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{RegisterTimeout: 60 * time.Millisecond, ProbeTimeout: 20 * time.Millisecond})
	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	v := h.waitForState(t, "den", StateUnreachable)
	if v.WakeFailures != 1 {
		t.Errorf("wakeFailures = %d, want 1", v.WakeFailures)
	}
	if v.Detail == "" {
		t.Error("a failed wake must say what happened")
	}
}

// ---- the zombie ----------------------------------------------------------

// THE WAKE PRE-FLIGHT. A dead provider still holding the capability slot will
// REFUSE the woken node's re-registration, so the slot is cleared BEFORE the
// machine is started — while it is still off — rather than at some later poll
// that may lose the race to the boot.
func TestWakeEvictsASilentProviderBeforeStartingTheMachine(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{ProbeTimeout: 20 * time.Millisecond, RegisterTimeout: time.Second, RegisterPollInterval: 5 * time.Millisecond})
	// A provider is registered and does not answer: the machine went away
	// without the socket closing.
	h.bus.set("", errors.New("call timed out"), true)

	evictedBeforeStart := false
	h.fly.onStart = func() {
		evictedBeforeStart = h.bus.evictionCount() > 0
		h.bus.set("den", nil, true) // the woken node registers
	}
	if _, err := h.sup.Wake(context.Background(), "den"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	if !evictedBeforeStart {
		t.Fatal("the stale provider was still holding the capability slot when the machine was started — " +
			"the woken node's re-registration would have been refused")
	}
	h.waitForState(t, "den", StateAvailable)
}

// A provider that IS answering is never evicted by the pre-flight.
func TestWakePreFlightDoesNotEvictALiveProvider(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den"), flyNode("sea")}, Tunables{ProbeTimeout: 20 * time.Millisecond, RegisterTimeout: 100 * time.Millisecond})
	h.bus.set("den", nil, true) // den's brain is live and answering
	h.sup.Reconcile(context.Background())
	_, _ = h.sup.Wake(context.Background(), "sea")
	if h.bus.evictionCount() != 0 {
		t.Error("the pre-flight evicted a provider that was answering")
	}
}

// The periodic poll is the passive half: a provider that goes silent is
// evicted after the configured number of strikes, so the slot is free before
// anybody presses anything.
func TestThePollEvictsASilentProviderAfterItsStrikes(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{ProbeTimeout: 20 * time.Millisecond, SilentStrikes: 2})
	h.bus.set("", errors.New("call timed out"), true)

	h.sup.Reconcile(context.Background())
	if h.bus.evictionCount() != 0 {
		t.Fatal("evicted on the first silent probe — one slow answer under load must not throw a live node off the bus")
	}
	h.sup.Reconcile(context.Background())
	if h.bus.evictionCount() != 1 {
		t.Fatalf("evictions = %d after two silent probes, want 1", h.bus.evictionCount())
	}
}

// "Nothing is registered" is NOT the zombie and must never evict: there is no
// slot to free, and an eviction here would be the hub closing a connection it
// did not identify.
func TestNoProviderNeverEvicts(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{SilentStrikes: 1})
	h.bus.set("", ErrNoProvider, false)
	h.sup.Reconcile(context.Background())
	h.sup.Reconcile(context.Background())
	if h.bus.evictionCount() != 0 {
		t.Errorf("evictions = %d with nothing registered, want 0", h.bus.evictionCount())
	}
}

// A silent probe must not be read as liveness: the node stays out of
// `available`.
func TestASilentProviderIsNeverReportedAvailable(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{ProbeTimeout: 20 * time.Millisecond, SilentStrikes: 99})
	h.fly.setState(flyapi.StateStarted)
	h.bus.set("", errors.New("call timed out"), true)
	h.sup.Reconcile(context.Background())
	if got := h.view(t, "den").State; got == string(StateAvailable) {
		t.Error("a registered-but-silent provider was reported as available")
	}
}

// ---- attribution ---------------------------------------------------------

func TestAttributionPrefersTheNodeTheBrainNames(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den"), flyNode("sea")}, Tunables{})
	h.bus.set("sea", nil, true)
	h.sup.Reconcile(context.Background())
	if got := h.view(t, "sea").State; got != string(StateAvailable) {
		t.Errorf("sea = %q, want available", got)
	}
	if got := h.view(t, "den").State; got == string(StateAvailable) {
		t.Error("den was reported available because ANOTHER node's brain answered")
	}
}

// With several nodes and an anonymous brain, the hub says nothing rather than
// guessing — a wrong guess renders a stopped machine as available.
func TestAnAnonymousBrainIsNotAttributedWhenSeveralNodesExist(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den"), flyNode("sea")}, Tunables{})
	h.fly.setState(flyapi.StateStopped)
	h.bus.set("", nil, true) // answered, named nobody
	h.sup.Reconcile(context.Background())
	for _, id := range []string{"den", "sea"} {
		if got := h.view(t, id).State; got == string(StateAvailable) {
			t.Errorf("%s was guessed available from an anonymous brain", id)
		}
	}
}

// With exactly one node there is only one possible answer, so an anonymous
// brain is attributed to it — which is what makes this work with a brain that
// has no WKS_NODE_ID set.
func TestAnAnonymousBrainIsAttributedWhenThereIsOnlyOneNode(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	h.bus.set("", nil, true)
	h.sup.Reconcile(context.Background())
	if got := h.view(t, "den").State; got != string(StateAvailable) {
		t.Errorf("state = %q, want available", got)
	}
}

// THE WAKE PATH IS HELD TO THE SAME REFUSAL, and it did not used to be.
//
// watchWake's poll condition carried an `|| pr.NodeID == ""` clause beside the
// attribution call. With one node it changed nothing — `attribute("")` already
// answers that node — so it was load-bearing in EXACTLY the multi-node case,
// which is the case where guessing is wrong. Two anonymous nodes, A up and B
// woken: the watcher read A's answer, matched the empty-id clause, and settled
// B as `available` with `wakeFailures` reset to 0 and A's exit record copied
// onto B's row, for a machine that may never have booted.
//
// Reconcile has refused to guess since the beginning (see the test above); this
// is the same refusal on the one path that spends money.
func TestAWakeIsNotSettledByANONYMOUSBrainWhenSeveralNodesExist(t *testing.T) {
	h := newHarness(t, []Node{flyNode("ord"), flyNode("iad")}, Tunables{
		RegisterTimeout:      80 * time.Millisecond,
		ProbeTimeout:         20 * time.Millisecond,
		RegisterPollInterval: 5 * time.Millisecond,
	})
	// ord is up, and its brain does NOT name itself — which is what every node
	// under deploy/ was before WKS_NODE_ID reached node/fly.toml's [env].
	h.bus.set("", nil, true)
	h.bus.setExit(&ExitRecord{Reason: "signal-TERM", At: "2026-08-25T10:00:00Z"})

	if _, err := h.sup.Wake(context.Background(), "iad"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	v := h.waitForState(t, "iad", StateUnreachable)
	if v.WakeFailures != 1 {
		t.Errorf("wakeFailures = %d, want 1 — the counter this arc names as the mitigation for a node that will not boot was reset by the anonymous answer", v.WakeFailures)
	}
	if v.LastSeen != 0 {
		t.Error("iad was credited with a liveness answer that belongs to ord")
	}
	if v.LastExit != nil {
		t.Errorf("iad carries lastExit %+v — that record is ord's, read off ord's volume, and it is now the only account of a machine that may never have booted", v.LastExit)
	}
}

// The other half, and it is why the clause could not simply be deleted without
// one: a node that DOES name itself still completes a wake with several nodes
// registered. WKS_NODE_ID is what buys this back, which is why it is now set in
// deploy/fly/node/fly.toml.
func TestAWakeCompletesWithSeveralNodesWhenTheBrainNamesItself(t *testing.T) {
	h := newHarness(t, []Node{flyNode("ord"), flyNode("iad")}, Tunables{
		RegisterTimeout:      time.Second,
		ProbeTimeout:         20 * time.Millisecond,
		RegisterPollInterval: 5 * time.Millisecond,
	})
	h.fly.onStart = func() { h.bus.set("iad", nil, true) }
	if _, err := h.sup.Wake(context.Background(), "iad"); err != nil {
		t.Fatalf("Wake: %v", err)
	}
	v := h.waitForState(t, "iad", StateAvailable)
	if v.WakeFailures != 0 {
		t.Errorf("wakeFailures = %d after a wake that succeeded, want 0", v.WakeFailures)
	}
	if got := h.view(t, "ord").State; got == string(StateAvailable) {
		t.Error("ord was reported available off the brain that named iad")
	}
}

// ---- events --------------------------------------------------------------

func TestStateChangesArePublishedWithTheirPreviousState(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())
	select {
	case c := <-h.ch:
		if c.View.State != string(StateAvailable) {
			t.Errorf("published state = %q, want available", c.View.State)
		}
		if c.Previous != StateUnreachable {
			t.Errorf("previous = %q, want unreachable", c.Previous)
		}
	case <-time.After(time.Second):
		t.Fatal("no change published")
	}
}

// An unchanged reading publishes nothing: the poll runs forever and must not
// be a permanent event source.
func TestAnUnchangedReadingPublishesNothing(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())
	<-h.ch // the first transition
	h.sup.Reconcile(context.Background())
	select {
	case c := <-h.ch:
		t.Fatalf("an unchanged reading published %+v", c)
	case <-time.After(50 * time.Millisecond):
	}
}

// Nothing a client can see, on ANY path, carries the credential.
func TestNoPublishedChangeCarriesTheToken(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{ProbeTimeout: 20 * time.Millisecond, RegisterTimeout: 50 * time.Millisecond})
	h.fly.startErr = &flyapi.APIError{Status: 502, Body: "gateway said: authorization: Bearer " + secretToken}
	_, _ = h.sup.Wake(context.Background(), "den")
	h.sup.Reconcile(context.Background())
	close(h.ch)
	for c := range h.ch {
		if containsSecret(c.View) {
			t.Fatalf("a published node change carried the Fly token: %+v", c.View)
		}
	}
	for _, v := range h.sup.List() {
		if containsSecret(v) {
			t.Fatalf("nodes.list carried the Fly token: %+v", v)
		}
	}
}
