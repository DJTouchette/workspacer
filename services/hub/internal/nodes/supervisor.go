package nodes

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/flyapi"
)

// ErrNoProvider is what [BusProbe.ProbeBrain] returns when NOBODY is
// registered as the brain's provider. It is a different answer from "a
// provider is registered and did not reply", and conflating the two is exactly
// the zombie the eviction path exists for.
var ErrNoProvider = errors.New("no provider for brain.info")

// ErrProbeUnavailable means the HUB could not ask — its own loopback bus
// client is not connected — as opposed to the node not answering. The
// difference matters: a probe failure that is our own fault must never be
// counted as a strike against a provider, because the eviction it would earn
// throws a live node off the bus.
var ErrProbeUnavailable = errors.New("the hub could not ask the bus")

// ErrUnknownNode is returned for an id that is not in the registry.
var ErrUnknownNode = errors.New("unknown node")

// BusProbe is the hub bus, as this package needs it. Three questions, and the
// third is a decision rather than a read.
type BusProbe interface {
	// ProbeBrain calls brain.info — the ONE capability only a brain ever
	// provides. It returns [ErrNoProvider] when nothing is registered, and
	// any other error when a provider IS registered but did not answer.
	ProbeBrain(ctx context.Context) (Probe, error)
	// BrainProviderRegistered reports whether some connection currently owns
	// brain.info.
	BrainProviderRegistered() bool
	// EvictBrainProvider force-closes the connection that owns brain.info and
	// releases the slot, returning whether there was one. See
	// bus.Server.EvictConn for why this cannot be left to TCP.
	EvictBrainProvider() bool
}

// Tunables are the supervisor's timings. Zero fields take the defaults.
type Tunables struct {
	// PollInterval is how often the hub takes a liveness reading.
	PollInterval time.Duration
	// ProbeTimeout bounds one brain.info call. A provider that does not answer
	// inside it is SILENT, which is a strike toward eviction — not an answer.
	ProbeTimeout time.Duration
	// RegisterTimeout is how long a wake waits for the node's provider to
	// register before the hub gives up on it. The brief's figure is ~90s.
	RegisterTimeout time.Duration
	// RegisterPollInterval is how often, once the machine is up, the hub asks
	// whether its provider has registered yet.
	RegisterPollInterval time.Duration
	// SilentStrikes is how many consecutive silent probes evict a provider.
	// Two rather than one, so a single slow answer under load does not throw
	// a live node off the bus.
	SilentStrikes int
	// StartRetries is how many EXTRA times a failed Fly start is retried.
	// One, per the brief. Fly's rate limit is 1 action/s/machine, so a retry
	// waits StartRetryDelay first.
	StartRetries    int
	StartRetryDelay time.Duration

	// StopSignal is what the hub sends a node it is putting to sleep. It is
	// NOT caller-chosen, and that is a security decision rather than a
	// simplification: a caller that could name the signal could name SIGKILL,
	// which skips claudemon's flush AND leaves the entrypoint no chance to
	// write the exit record — the one artefact that distinguishes a deliberate
	// sleep from a crash on the next wake. See [Supervisor.Sleep].
	StopSignal string
	// StopGrace is the DRAIN WINDOW the cloud API is told to allow between the
	// signal and its own SIGKILL. It is sent explicitly on every stop because
	// fly.toml's kill_timeout governs a PLATFORM stop and is never read by an
	// API-issued one — see flyapi.Client.Stop.
	StopGrace time.Duration
	// StopTimeout bounds the hub's own wait for the machine to reach
	// `stopped`. Longer than StopGrace, because the drain window starts when
	// the cloud API delivers the signal and not when the hub asked.
	StopTimeout time.Duration

	// KeepFailedWakesRunning turns OFF the one automatic stop in this package.
	//
	// A wake that starts a machine and never gets a provider used to leave that
	// machine running and billing with no way to stop it from inside the app —
	// the known cost of a wake-only v1, and the reason this field's zero value
	// is the safe one. The hub now stops what its own wake started, bounded to
	// exactly that: a machine THIS hub started, inside THIS wake's window,
	// whose provider never registered.
	//
	// It is NOT an idle timer and must not become one. Nothing here stops a
	// machine that is working, and nothing here stops a machine on a clock.
	// The escape hatch exists because a node that dies on boot is sometimes
	// worth leaving up to look at.
	KeepFailedWakesRunning bool
}

func (t Tunables) withDefaults() Tunables {
	if t.PollInterval <= 0 {
		t.PollInterval = 30 * time.Second
	}
	if t.ProbeTimeout <= 0 {
		t.ProbeTimeout = 8 * time.Second
	}
	if t.RegisterTimeout <= 0 {
		t.RegisterTimeout = 90 * time.Second
	}
	if t.RegisterPollInterval <= 0 {
		t.RegisterPollInterval = 2 * time.Second
	}
	if t.RegisterPollInterval > t.RegisterTimeout {
		t.RegisterPollInterval = t.RegisterTimeout / 4
	}
	if t.SilentStrikes <= 0 {
		t.SilentStrikes = 2
	}
	if t.StartRetries < 0 {
		t.StartRetries = 0
	}
	if t.StartRetries == 0 {
		t.StartRetries = 1
	}
	if t.StartRetryDelay <= 0 {
		t.StartRetryDelay = 2 * time.Second
	}
	if t.StopSignal == "" {
		// SIGTERM rather than SIGKILL, deliberately: the node's entrypoint
		// traps INT and TERM, flushes, and writes /data/state/last-exit.json
		// with reason "signal-TERM". A SIGKILLed node writes nothing, and the
		// hub's next wake then cannot tell that sleep from a crash.
		t.StopSignal = "SIGTERM"
	}
	if t.StopGrace <= 0 {
		// The node's fly.toml allows 60s for a platform stop; an API stop gets
		// its own window and this is it. Long enough for claudemon to flush,
		// short enough that a stuck node still stops billing.
		t.StopGrace = 45 * time.Second
	}
	if t.StopTimeout <= 0 {
		t.StopTimeout = 2 * time.Minute
	}
	if t.StopTimeout < t.StopGrace {
		t.StopTimeout = t.StopGrace
	}
	return t
}

// upness is what a transition ASSERTS about whether the machine is powered on,
// as opposed to whether its provider is answering. Three values, because the
// hub genuinely has all three answers and collapsing the third into either of
// the others is a lie:
//
//   - upYes / upNo   — this reading knows.
//   - upUnknown      — this reading LEARNED NOTHING about the power (the cloud
//     API did not answer, or answered something we do not
//     recognise), so the previous belief stands.
//
// It is a REQUIRED parameter of [Supervisor.setLocked] rather than a field
// somebody updates nearby, so a new transition cannot inherit a stale answer.
// That direction is the same one NodeView takes: name what goes in.
type upness int

const (
	upUnknown upness = iota
	upYes
	upNo
)

// Probe is what one brain.info answer tells the hub.
type Probe struct {
	// NodeID is the node the answering brain named (WKS_NODE_ID). Empty when
	// it named none, which is every brain not running on a registered node.
	NodeID string
	// LastExit is the node's own record of how its PREVIOUS run ended. Nil
	// when the brain is not on a node, or the node has never exited.
	LastExit *ExitRecord
}

// Change is one node's transition, as published on the bus.
type Change struct {
	View     NodeView
	Previous State
}

// state is the hub's live belief about one node. NONE OF IT IS PERSISTED.
// That is deliberate and it is the brief's own rule: in-memory state must not
// be trusted across a hub restart, so there is no file to be tempted by. The
// hub boots believing nothing and reconciles against the cloud API — see
// [Supervisor.Reconcile].
type state struct {
	state        State
	since        time.Time
	lastSeen     time.Time
	detail       string
	wakeFailures int
	// lastExit is the node's own account of how its PREVIOUS run ended, read
	// on attachment. See lastexit.go for why the hub cannot get this from the
	// cloud API, and for the honest limit on when it is readable.
	lastExit *ExitRecord

	// mayBeRunning is what the hub believes about the MACHINE's power, which is
	// a different question from whether its provider answers — and the whole
	// reason the sleep path needs it: `unreachable` covers both "running and
	// providing nothing" (a meter, and the case a Sleep button exists for) and
	// "stopped and something is wrong" (nothing to switch off). A client that
	// cannot tell those apart offers a dead button for the second.
	//
	// It is on the wire rather than inferred from the detail sentence. Sniffing
	// prose for "billing" was the first shape of this and it was wrong on its
	// first real input: the detail for a machine the hub had ALREADY stopped
	// reads "…so it would not keep billing", which a regex reads as a running
	// machine. A fact the hub holds belongs in a field.
	mayBeRunning bool

	// sleptByHub records that THIS hub process issued the stop that put this
	// machine to sleep. It is the hub's own half of the crash-vs-sleep answer
	// and the only half readable while the node is OFF (lastExit lives on the
	// node's volume). Cleared on every wake, and never persisted — a restarted
	// hub did not issue that stop and must not claim it did.
	sleptByHub bool

	// waking marks a wake in flight, which makes nodes.wake idempotent: three
	// clients tapping the button produce ONE Fly start call, not three, and
	// three would earn a 429 (Fly allows one action per second per machine).
	waking       bool
	wakeDeadline time.Time

	// stopping marks a sleep in flight, and makes nodes.sleep idempotent for
	// the same reason waking does for nodes.wake.
	stopping     bool
	stopDeadline time.Time

	// gen is bumped by EVERY transition a caller asks for (a wake, a sleep).
	// A watcher goroutine captures it and refuses to write anything once it
	// has moved on — without it, a wake watcher whose probe succeeds a beat
	// after somebody pressed Sleep would report `available` for a machine that
	// is draining, and the state map would then disagree with the machine for
	// as long as nobody looked.
	gen int
}

// Supervisor owns the registry and its state machine.
type Supervisor struct {
	nodes   []Node
	byID    map[string]Node
	clients map[string]flyapi.Client // node id → client; absent = not wakeable
	bus     BusProbe
	tun     Tunables
	now     func() time.Time
	logf    func(string, ...any)
	publish func(Change)

	mu            sync.Mutex
	st            map[string]*state
	silentStrikes int
	reconciled    bool
}

// Options configures a Supervisor.
type Options struct {
	Nodes []Node
	Bus   BusProbe
	// Clients maps node id → Fly client. A node absent from this map is
	// observable but not wakeable — either it has no coordinates, or the hub
	// found no credential for it.
	Clients  map[string]flyapi.Client
	Tunables Tunables
	Now      func() time.Time
	Logf     func(string, ...any)
	// Publish is called on every state or detail change. Nil disables events.
	Publish func(Change)
}

// New builds a supervisor. Every node starts UNREACHABLE and unreconciled,
// which is the honest answer before the first reading: the hub has just
// started and genuinely does not know.
func New(o Options) *Supervisor {
	s := &Supervisor{
		nodes:   append([]Node(nil), o.Nodes...),
		byID:    make(map[string]Node, len(o.Nodes)),
		clients: o.Clients,
		bus:     o.Bus,
		tun:     o.Tunables.withDefaults(),
		now:     o.Now,
		logf:    o.Logf,
		publish: o.Publish,
		st:      make(map[string]*state, len(o.Nodes)),
	}
	if s.now == nil {
		s.now = time.Now
	}
	if s.logf == nil {
		s.logf = func(string, ...any) {}
	}
	if s.clients == nil {
		s.clients = map[string]flyapi.Client{}
	}
	for _, n := range s.nodes {
		s.byID[n.ID] = n
		s.st[n.ID] = &state{
			state:  StateUnreachable,
			since:  s.now(),
			detail: "not reconciled yet — the hub has just started and has not asked the cloud API anything",
			// A booted hub has not asked anything, so it does not know whether
			// the machine is up. False rather than unknown-on-the-wire because
			// the wire bit means "the hub believes this is running", and it
			// does not believe anything yet.
			mayBeRunning: false,
		}
	}
	return s
}

// Len reports how many nodes are registered.
func (s *Supervisor) Len() int { return len(s.nodes) }

// wakeableNow reports whether the hub holds BOTH coordinates and a credential.
func (s *Supervisor) wakeableNow(id string) bool {
	_, ok := s.clients[id]
	return ok && s.byID[id].Wakeable()
}

// List renders every node, in registry order (which is the order a person
// wrote them in, so two readings of an unchanged hub render identically).
func (s *Supervisor) List() []NodeView {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.listLocked()
}

func (s *Supervisor) listLocked() []NodeView {
	out := make([]NodeView, 0, len(s.nodes))
	for _, n := range s.nodes {
		out = append(out, s.viewLocked(n))
	}
	return out
}

func (s *Supervisor) viewLocked(n Node) NodeView {
	st := s.st[n.ID]
	if st == nil {
		st = &state{state: StateUnreachable}
	}
	v := ViewOf(n, st.state, st.since, st.lastSeen, st.detail, st.wakeFailures)
	v.LastExit = st.lastExit
	v.SleptByHub = st.sleptByHub
	v.MayBeRunning = st.mayBeRunning
	// The record's own Wakeable() only knows about coordinates. A node with
	// coordinates and no credential cannot be woken, and telling a client it
	// can is how you ship a button that fails every time.
	v.Wakeable = v.Wakeable && s.wakeableNow(n.ID)
	return v
}

// View renders one node.
func (s *Supervisor) View(id string) (NodeView, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	n, ok := s.byID[id]
	if !ok {
		return NodeView{}, fmt.Errorf("%w: %q", ErrUnknownNode, id)
	}
	return s.viewLocked(n), nil
}

// setLocked moves a node to a state, returning the change to publish (nil when
// nothing a client can see actually changed).
func (s *Supervisor) setLocked(id string, next State, detail string, up upness) *Change {
	st := s.st[id]
	if st == nil {
		return nil
	}
	wasRunning := st.mayBeRunning
	switch up {
	case upYes:
		st.mayBeRunning = true
	case upNo:
		st.mayBeRunning = false
	}
	if st.state == next && st.detail == detail && st.mayBeRunning == wasRunning {
		return nil
	}
	prev := st.state
	if st.state != next {
		st.since = s.now()
	}
	st.state, st.detail = next, detail
	return &Change{View: s.viewLocked(s.byID[id]), Previous: prev}
}

func (s *Supervisor) emit(changes ...*Change) {
	if s.publish == nil {
		return
	}
	for _, c := range changes {
		if c != nil {
			s.publish(*c)
		}
	}
}
