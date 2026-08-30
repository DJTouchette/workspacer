package main

// The hub-side wiring for limit-aware routing: read claudemon's usage document
// on the tick, hold the latest one, and hand it to whoever is deciding.
//
// It lives in the hub, in Go, WITH NO TYPESCRIPT TWIN, for the reason
// cmd/hub/quiescence.go states for the fleet signal: every input the routing
// layer reads is either the hub's own or arrives over the bus, and the hub is
// the one process that exists in every deployment. A limit reading reaches a
// client as an ANSWER, never as a recomputation. There must be no routing
// handler in cmd/brain and no routing service in the desktop; the moment there
// is one, this becomes a byte-pinned twin and inherits the repo's most
// expensive failure mode.
//
// This file is the WIRING half. The rule and the shapes are pure and live in
// internal/limits — same split as internal/quiescence and cmd/hub/quiescence.go.
//
// Everything here is READ-ONLY, on both ends. It makes one HTTP GET against
// claudemon (the --claudemon flag, default http://127.0.0.1:7891) and publishes
// nothing. Routing exposes no write RPC over the bus, ever.

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// usageProbeTimeout bounds one fetch. claudemon answers /usage/report from what
// each CLI already left on disk, with nothing running and no network call, so
// this is generous — but a daemon that does not answer must make the reading
// UNKNOWN rather than block the tick.
const usageProbeTimeout = 10 * time.Second

// usageSampleIdleAfter is how long the poller keeps taking readings after the
// last time anybody asked for one.
//
// Straight from fleetWatcher's discipline, and for the same reason: nothing
// should poll /usage/report on a machine nobody has asked for a routing
// decision on. On an ordinary desktop install nothing consults routing at all,
// and a permanent background poll for an answer no one reads is exactly the
// ambient cost this layer is supposed to remove. The first ask starts the
// poller and it winds down again once the asking stops.
const usageSampleIdleAfter = 15 * time.Minute

// usageMaxAge is how stale a held document may be before an ask refetches it
// inline rather than answering from the cache.
//
// It is deliberately larger than the sample interval and smaller than any
// window: a rate-limit window moves over hours, so a reading a minute old is
// fine, and a reading five minutes old on a poller that has just started is
// not worth blocking a decision for. Note that STALENESS OF THE DOCUMENT AND
// CURRENCY OF A WINDOW ARE DIFFERENT QUESTIONS — a two-second-old document can
// contain a window that lapsed two days ago, which is why limits.Snapshot is
// re-judged against the caller's clock on every use and this constant does not
// enter that decision at all.
const usageMaxAge = 3 * time.Minute

// usageWatcher polls claudemon's usage document and holds the latest one.
//
// It holds the document UNJUDGED. Every verdict is produced by
// limits.Snapshot.Buckets(now) at the moment of the decision, because the whole
// defect this layer exists to close is a window verdict outliving the instant
// it was correct at — including the daemon's own `is_current`, which is
// computed at generated_at and is stale the moment a client caches the
// document, which is precisely what this type does.
type usageWatcher struct {
	// base is the claudemon API base URL (the --claudemon flag).
	base string
	http *http.Client

	mu       sync.Mutex
	snap     limits.Snapshot
	haveSnap bool
	err      error
	lastAsk  time.Time
	// inflight is the fetch currently in progress, shared by every caller that
	// arrives while it runs. See [usageWatcher.refresh].
	inflight *usageFetch
}

// usageFetch is one in-flight GET /usage/report and its result, shared by every
// caller that asked while it was running.
type usageFetch struct {
	// started is when the request went out, so a joiner can be told how old the
	// reading it is about to share will be.
	started time.Time
	done    chan struct{}
	snap    limits.Snapshot
	err     error
}

func newUsageWatcher(base string) *usageWatcher {
	return &usageWatcher{
		base: base,
		http: &http.Client{Timeout: usageProbeTimeout},
	}
}

// noteAsk records that somebody wants a routing answer, which is what starts
// and keeps the poller running.
func (w *usageWatcher) noteAsk(now time.Time) {
	w.mu.Lock()
	w.lastAsk = now
	w.mu.Unlock()
}

// sampling reports whether anybody is currently using the signal.
func (w *usageWatcher) sampling(now time.Time) bool {
	w.mu.Lock()
	defer w.mu.Unlock()
	return !w.lastAsk.IsZero() && now.Sub(w.lastAsk) <= usageSampleIdleAfter
}

// run takes a reading every interval until ctx ends, while anybody is asking.
func (w *usageWatcher) run(ctx context.Context, interval time.Duration) {
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
			_, _ = w.refresh(ctx)
		}
	}
}

// refresh takes one reading and replaces the held one, SHARING a fetch that is
// already in progress rather than starting a second.
//
// THE SINGLEFLIGHT IS WHY THIS EXISTS IN THIS SHAPE. Every routing decision
// takes its own reading (usageDecisionMaxAge is 0), which was affordable while
// routing was advisory and rarely asked. It stops being affordable the moment
// dispatch routes through it: five concurrent decisions used to be five
// concurrent GETs against one loopback daemon, each one booting nothing but each
// one queued behind the others' work. Now they are one request with five
// waiters.
//
// SHARING A CONCURRENT FETCH IS NOT CACHING, and the distinction is the whole
// feature. A cache serves a document that was correct WHEN IT WAS FETCHED and is
// being read at some later, unrelated instant — that is the founding defect this
// layer exists to close. A joiner here waits for a request that is still open,
// so the document it receives is bounded by usageProbeTimeout, not by how long
// ago somebody else happened to ask. Nothing is ever served from a completed
// fetch: the moment one lands, the next ask starts a new one.
//
// THE FETCH IS DETACHED FROM THE CALLER'S CONTEXT, deliberately. A waiter that
// gives up (see usageDecisionWait) must not cancel a request other waiters are
// still on, and the reading should still land for the next asker rather than
// being thrown away at the finish line. The request bounds itself with
// usageProbeTimeout instead.
//
// A FAILED FETCH REPLACES THE DOCUMENT WITH THE ERROR rather than leaving the
// last good one in place. Keeping a stale document alive across an outage is
// the same defect one level up: the reading would keep answering with numbers
// nobody can vouch for, and the routing layer's honest answer to "the daemon is
// not reachable" is UNKNOWN, not "whatever it said last time".
func (w *usageWatcher) refresh(ctx context.Context) (limits.Snapshot, error) {
	w.mu.Lock()
	call := w.inflight
	if call == nil {
		call = &usageFetch{started: time.Now(), done: make(chan struct{})}
		w.inflight = call
		go w.runFetch(call)
	}
	w.mu.Unlock()

	select {
	case <-call.done:
		return call.snap, call.err
	case <-ctx.Done():
		// The caller's own patience ran out. The fetch keeps going and its result
		// lands for whoever asks next; this caller gets UNKNOWN with the reason
		// named, which is what capacityFor turns into an explained decision
		// rather than a refusal.
		return limits.Snapshot{}, fmt.Errorf(
			"claudemon's usage report did not arrive within this decision's %s budget (the fetch is still open and will land for the next ask): %w",
			time.Since(call.started).Round(time.Millisecond), ctx.Err())
	}
}

// runFetch is the single owner of one shared fetch: it takes the reading, stores
// it, releases the waiters, and clears the slot so the NEXT ask starts a fresh
// request instead of reusing this one's answer.
func (w *usageWatcher) runFetch(call *usageFetch) {
	ctx, cancel := context.WithTimeout(context.Background(), usageProbeTimeout)
	defer cancel()
	snap, err := w.fetch(ctx)

	w.mu.Lock()
	if err != nil {
		w.err, w.haveSnap = err, false
	} else {
		w.snap, w.haveSnap, w.err = snap, true, nil
	}
	call.snap, call.err = snap, err
	w.inflight = nil
	w.mu.Unlock()

	close(call.done)
}

func (w *usageWatcher) fetch(ctx context.Context) (limits.Snapshot, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, w.base+"/usage/report", nil)
	if err != nil {
		return limits.Snapshot{}, err
	}
	resp, err := w.http.Do(req)
	if err != nil {
		return limits.Snapshot{}, fmt.Errorf("claudemon GET /usage/report: %w", err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
	if err != nil {
		return limits.Snapshot{}, fmt.Errorf("claudemon GET /usage/report: %w", err)
	}
	if resp.StatusCode >= 400 {
		return limits.Snapshot{}, fmt.Errorf("claudemon GET /usage/report: %s", resp.Status)
	}
	return limits.DecodeReport(body, time.Now())
}

// usageDecisionMaxAge is how old a held document may be at the moment a ROUTING
// DECISION is made, and it is ZERO on purpose: a decision takes its own reading.
//
// It is deliberately NOT usageMaxAge, and the two answer different questions. A
// reading a minute old is fine for a status pane and is what usageMaxAge is
// sized for. A routing decision is a rare, consequential act that commits an
// hour of a frontier model's allowance, and the one thing that makes its
// evidence trustworthy is that nothing has moved since it was gathered — the
// currency guard fixes a window verdict outliving its instant, and this fixes
// the DOCUMENT outliving its instant, which is the same defect one level out. A
// window that reset thirty seconds ago is unreadable in a document fetched two
// minutes ago and readable in one fetched now.
//
// The cost is one loopback GET against a document claudemon assembles from what
// each CLI already wrote to disk — no network call, nothing spawned. That is a
// price worth paying per decision, and the poller and its cache remain for
// every other reader.
const usageDecisionMaxAge = 0

// usageDecisionWait is the longest a ROUTING DECISION waits for that reading
// before answering without one.
//
// A DECISION MUST NOT BE ABLE TO HANG. Routing is now on the dispatch path — a
// manager asks routing.select and then spawns — so an unresponsive claudemon
// must degrade the ANSWER, never stall the fleet. usageProbeTimeout (10s) bounds
// the HTTP request and is the right size for a background poll; it is far too
// long to keep a dispatch waiting, and a hung TCP connect can sit there for the
// whole of it.
//
// WHAT A DECISION DOES WHEN THE READING IS UNAVAILABLE, decided deliberately:
// it routes as if capacity were UNKNOWN, and says so in the reason list. Not a
// refusal — a hub that would not dispatch because a usage daemon was restarting
// would be strictly worse than the advisory layer it replaced. Not a healthy
// default either: UNKNOWN goes through the matrix's own `providers[].when_unknown`
// (`yellow` for the metered providers), and UNKNOWN satisfies neither the
// spend-down arm nor the conserve-by-forecast arm, so an unreachable daemon
// produces NORMAL mode — no promotion off evidence that does not exist, and no
// phantom conservation either. The observed health stays UNKNOWN on the answer
// next to the assumed one, so nothing downstream can read "we could not ask" as
// "it is fine".
const usageDecisionWait = 3 * time.Second

// Latest is the ask. It notes the ask (which starts the poller), and answers
// from the held document unless there is none or it has aged past usageMaxAge,
// in which case it takes one reading inline — so the FIRST ask on a quiet
// machine gets a real answer instead of an empty one it would have to explain
// away.
//
// The returned Snapshot is unjudged: call Buckets with the deciding instant.
func (w *usageWatcher) Latest(ctx context.Context) (limits.Snapshot, error) {
	return w.LatestWithin(ctx, usageMaxAge)
}

// LatestWithin is Latest with the caller's own freshness bound. A maxAge of 0
// or less always takes a reading; see usageDecisionMaxAge.
func (w *usageWatcher) LatestWithin(ctx context.Context, maxAge time.Duration) (limits.Snapshot, error) {
	now := time.Now()
	w.noteAsk(now)

	w.mu.Lock()
	fresh := maxAge > 0 && w.haveSnap && now.Sub(w.snap.FetchedAt) < maxAge
	snap, err := w.snap, w.err
	w.mu.Unlock()
	if fresh {
		return snap, nil
	}

	// The result of THIS fetch (or of the one it joined), not whatever landed in
	// the held document afterwards. Reading the field back would race a
	// concurrent failed fetch onto a successful caller's answer, which is the
	// same "correct when it was taken, wrong when it was read" shape one level
	// smaller.
	snap, err = w.refresh(ctx)
	if err != nil {
		return limits.Snapshot{}, err
	}
	return snap, nil
}
