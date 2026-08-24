package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/jobs"
	"github.com/djtouchette/workspacer-hub/internal/quiescence"
)

// fleetRig stands up a real hub bus with the watcher wired to its own loopback
// client, so a reading takes the whole path it takes in production: an actual
// bus call for the session rows, actual live connections for the client list.
// Only the fleet's CONTENT is faked.
type fleetRig struct {
	srv     *bus.Server
	watcher *fleetWatcher
	busURL  string

	mu   sync.Mutex
	rows json.RawMessage
	// serveFleet false = no provider answers, the case a fail-open would be
	// worst in.
	serveFleet bool
}

func newFleetRig(t *testing.T, serveFleet bool) *fleetRig {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)

	rig := &fleetRig{rows: json.RawMessage(`[]`), serveFleet: serveFleet}
	srv := bus.NewServer(broker.New())
	if serveFleet {
		srv.RegisterLocal("sessions.snapshots", func(json.RawMessage) (any, error) {
			rig.mu.Lock()
			defer rig.mu.Unlock()
			return rig.rows, nil
		})
	}
	key := newInternalKey()
	if key == "" {
		t.Fatal("no internal key")
	}
	srv.SetInternalKey(key)
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	rig.busURL = strings.Replace(hs.URL, "http://", "ws://", 1) + "/bus"
	rig.srv = srv

	self := busclient.New(bus.InternalDialURL(rig.busURL, key), "")
	go self.Run(ctx)
	waitReady(t, self)

	rig.watcher = newFleetWatcher(srv, self)
	srv.RegisterLocalIdent("fleet.quiescence", rig.watcher.answer)
	return rig
}

func (r *fleetRig) setFleet(raw string) {
	r.mu.Lock()
	r.rows = json.RawMessage(raw)
	r.mu.Unlock()
}

func waitReady(t *testing.T, c *busclient.Client) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if c.Ready() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("bus client never connected")
}

func kindsOf(bs []quiescence.Blocker) []string {
	out := make([]string, 0, len(bs))
	for _, b := range bs {
		out = append(out, b.Kind)
	}
	return out
}

func hasKind(bs []quiescence.Blocker, kind string) bool {
	for _, b := range bs {
		if b.Kind == kind {
			return true
		}
	}
	return false
}

// A live session mid-turn blocks, end to end, over the real bus.
func TestWatcherReadsTheFleetOverTheBus(t *testing.T) {
	rig := newFleetRig(t, true)
	rig.setFleet(`[{"session_id":"s1","mode":"responding"}]`)

	res := rig.watcher.mon.Observe(rig.watcher.read(context.Background()))
	if res.Quiescent {
		t.Fatal("quiescent with a session mid-turn")
	}
	if !hasKind(res.Blockers, quiescence.KindSessionWorking) {
		t.Fatalf("want a session-working blocker, got %v", kindsOf(res.Blockers))
	}
}

// A SPAWNING session — claudemon's `unknown` mode, which is also where a
// terminal PTY lives permanently — must block, and the blocker must say why in
// terms an operator can act on.
func TestWatcherBlocksOnASpawningSession(t *testing.T) {
	rig := newFleetRig(t, true)
	rig.setFleet(`[{"session_id":"s1","mode":"unknown"}]`)

	res := rig.watcher.mon.Observe(rig.watcher.read(context.Background()))
	if res.Quiescent {
		t.Fatal("quiescent with a session that has not reported yet")
	}
	if !hasKind(res.Blockers, quiescence.KindSessionUnknown) {
		t.Fatalf("want a session-unknown blocker, got %v", kindsOf(res.Blockers))
	}
	found := false
	for _, b := range res.Blockers {
		if strings.Contains(b.Detail, "TERMINAL") && strings.Contains(b.Detail, "SPAWNING") {
			found = true
		}
	}
	if !found {
		t.Errorf("the blocker does not name what a session in this state might be: %+v", res.Blockers)
	}
}

// The provider seam failing must BLOCK. A hub that cannot reach any session
// provider knows nothing about the fleet, and "I could not ask" is not
// "nothing is running".
func TestWatcherBlocksWhenNoProviderAnswers(t *testing.T) {
	rig := newFleetRig(t, false)
	res := rig.watcher.mon.Observe(rig.watcher.read(context.Background()))
	if res.Quiescent {
		t.Fatal("quiescent with NO session provider on the bus at all")
	}
	if !hasKind(res.Blockers, quiescence.KindFleetUnreadable) {
		t.Fatalf("want a fleet-unreadable blocker, got %v", kindsOf(res.Blockers))
	}
}

// The hub's own machinery must not read as somebody using the machine.
// Without this the signal defeats itself: the sampler's loopback client is
// permanently connected, and the jobs runner shares it.
func TestTheHubsOwnClientIsNotAUser(t *testing.T) {
	rig := newFleetRig(t, true)
	rig.setFleet(`[{"session_id":"s1","mode":"input"}]`)

	in := rig.watcher.read(context.Background())
	if len(in.Clients) != 0 {
		t.Fatalf("expected no user-facing clients, got %+v", in.Clients)
	}
	if bs := quiescence.Evaluate(in, quiescence.Tunables{}); len(bs) != 0 {
		t.Fatalf("a fleet with only infrastructure connected must be calm, got %v", kindsOf(bs))
	}
}

// The poller is a bus client. A connection whose most recent act was asking
// this question has done nothing, and must not be the reason the answer is no.
func TestAskingDoesNotMakeTheMachineBusy(t *testing.T) {
	rig := newFleetRig(t, true)
	rig.setFleet(`[{"session_id":"s1","mode":"input"}]`)

	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	caller := busclient.New(rig.busURL, "")
	go caller.Run(ctx)
	waitReady(t, caller)

	// Fresh connection, nothing asked yet: somebody just opened something.
	in := rig.watcher.read(ctx)
	if len(in.Clients) != 1 {
		t.Fatalf("expected the fresh connection to count, got %+v", in.Clients)
	}
	if bs := quiescence.Evaluate(in, quiescence.Tunables{}); !hasKind(bs, quiescence.KindClientActive) {
		t.Fatalf("a client that just connected should block, got %v", kindsOf(bs))
	}

	// It asks. That is not use.
	if _, err := caller.Call(ctx, "fleet.quiescence", map[string]any{}); err != nil {
		t.Fatalf("call: %v", err)
	}
	if in = rig.watcher.read(ctx); len(in.Clients) != 0 {
		t.Fatalf("the poller still counted as a user after asking: %+v", in.Clients)
	}

	// Anything ELSE it does counts again.
	if _, err := caller.Call(ctx, "sessions.snapshots", map[string]any{}); err != nil {
		t.Fatalf("call: %v", err)
	}
	if in = rig.watcher.read(ctx); len(in.Clients) != 1 {
		t.Fatalf("a poller that went on to do real work must count again: %+v", in.Clients)
	}
}

// The job schedule reaches the predicate, and the shell exemption that makes a
// shell-action poller possible is wired rather than merely written down.
func TestJobScheduleReachesThePredicate(t *testing.T) {
	rig := newFleetRig(t, true)
	rig.setFleet(`[{"session_id":"s1","mode":"input"}]`)
	soon := time.Now().Add(2 * time.Minute)
	rig.watcher.jobsFn = func() []jobs.Scheduled {
		return []jobs.Scheduled{
			{ID: "j1", Name: "nightly review", ActionKind: "spawn", NextRun: soon},
			{ID: "j2", Name: "check for quiet", ActionKind: "shell", NextRun: soon},
		}
	}
	bs := quiescence.Evaluate(rig.watcher.read(context.Background()), quiescence.Tunables{})
	if !hasKind(bs, quiescence.KindJobDueSoon) {
		t.Fatalf("a spawn job due in two minutes must block, got %v", kindsOf(bs))
	}
	for _, b := range bs {
		if b.ID == "j2" {
			t.Errorf("the shell poller blocked itself: %+v", b)
		}
	}
}

// The method answers over the wire in the shape the CLI and any script parse,
// and a fresh hub refuses rather than guessing.
func TestFleetQuiescenceAnswersOverTheBus(t *testing.T) {
	rig := newFleetRig(t, true)
	rig.setFleet(`[{"session_id":"s1","mode":"input"}]`)
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	caller := busclient.New(rig.busURL, "")
	go caller.Run(ctx)
	waitReady(t, caller)

	raw, err := caller.Call(ctx, "fleet.quiescence", map[string]any{})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	var got quiescence.Result
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatalf("unreadable answer %s: %v", raw, err)
	}
	if got.Quiescent {
		t.Fatal("a hub that has never taken a reading must not answer yes")
	}
	if !hasKind(got.Blockers, quiescence.KindStaleSample) {
		t.Fatalf("want a stale-sample refusal, got %v", kindsOf(got.Blockers))
	}
	if got.Since != nil {
		t.Errorf("since must be null when the answer is no, got %v", *got.Since)
	}
	if got.DwellSeconds <= 0 {
		t.Error("the answer does not say how long the dwell is, so a caller cannot tell how close it is")
	}
}

// Nobody who is not using the signal should pay for it. The sampler's whole
// cost is a session-snapshot call per tick, and on an ordinary desktop install
// nothing ever asks — a permanent background poll for an answer no one reads
// is the ambient cost this feature exists to remove, not to add.
func TestTheSamplerOnlyRunsWhileSomebodyIsAsking(t *testing.T) {
	rig := newFleetRig(t, true)
	now := time.Now()

	if rig.watcher.sampling(now) {
		t.Fatal("a hub nobody has ever asked is sampling")
	}
	rig.watcher.noteAsk(0)
	if !rig.watcher.sampling(time.Now()) {
		t.Fatal("an ask did not start the sampler")
	}
	if rig.watcher.sampling(time.Now().Add(sampleIdleAfter + time.Minute)) {
		t.Fatal("the sampler kept running long after the last ask")
	}
	// A poller at any sensible interval keeps it warm.
	if !rig.watcher.sampling(time.Now().Add(5 * time.Minute)) {
		t.Fatal("a five-minute poller would find the sampler dormant")
	}
}
