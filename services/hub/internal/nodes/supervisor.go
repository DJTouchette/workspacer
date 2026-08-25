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
	return t
}

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

	// waking marks a wake in flight, which makes nodes.wake idempotent: three
	// clients tapping the button produce ONE Fly start call, not three, and
	// three would earn a 429 (Fly allows one action per second per machine).
	waking       bool
	wakeDeadline time.Time
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
func (s *Supervisor) setLocked(id string, next State, detail string) *Change {
	st := s.st[id]
	if st == nil {
		return nil
	}
	if st.state == next && st.detail == detail {
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
