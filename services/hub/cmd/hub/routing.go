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
			w.refresh(ctx)
		}
	}
}

// refresh takes one reading and replaces the held one.
//
// A FAILED FETCH REPLACES THE DOCUMENT WITH THE ERROR rather than leaving the
// last good one in place. Keeping a stale document alive across an outage is
// the same defect one level up: the reading would keep answering with numbers
// nobody can vouch for, and the routing layer's honest answer to "the daemon is
// not reachable" is UNKNOWN, not "whatever it said last time".
func (w *usageWatcher) refresh(ctx context.Context) {
	cctx, cancel := context.WithTimeout(ctx, usageProbeTimeout)
	defer cancel()
	snap, err := w.fetch(cctx)

	w.mu.Lock()
	defer w.mu.Unlock()
	if err != nil {
		w.err, w.haveSnap = err, false
		return
	}
	w.snap, w.haveSnap, w.err = snap, true, nil
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

// Latest is the ask. It notes the ask (which starts the poller), and answers
// from the held document unless there is none or it has aged past usageMaxAge,
// in which case it takes one reading inline — so the FIRST ask on a quiet
// machine gets a real answer instead of an empty one it would have to explain
// away.
//
// The returned Snapshot is unjudged: call Buckets with the deciding instant.
func (w *usageWatcher) Latest(ctx context.Context) (limits.Snapshot, error) {
	now := time.Now()
	w.noteAsk(now)

	w.mu.Lock()
	fresh := w.haveSnap && now.Sub(w.snap.FetchedAt) < usageMaxAge
	snap, err := w.snap, w.err
	w.mu.Unlock()
	if fresh {
		return snap, nil
	}

	w.refresh(ctx)

	w.mu.Lock()
	defer w.mu.Unlock()
	if !w.haveSnap {
		if w.err != nil {
			return limits.Snapshot{}, w.err
		}
		if err != nil {
			return limits.Snapshot{}, err
		}
		return limits.Snapshot{}, fmt.Errorf("no usage reading")
	}
	return w.snap, nil
}
