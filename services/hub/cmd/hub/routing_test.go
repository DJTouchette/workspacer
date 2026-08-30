package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

// The document served here is the SAME real capture internal/limits'
// bucket_test.go reads — a live GET /usage/report from 2026-08-30 with home
// directories rewritten. Using the real body rather than a hand-written stub is
// the point: the shapes that break a decoder are the ones nobody would think to
// write down (an account key that is null, three claude logins at once, a
// provider whose every window is permanently unavailable).
func liveReportBody(t *testing.T) []byte {
	t.Helper()
	p := filepath.Join("..", "..", "internal", "limits", "testdata", "usage-report.json")
	raw, err := os.ReadFile(p)
	if err != nil {
		t.Fatalf("read %s: %v", p, err)
	}
	return raw
}

func TestUsageWatcherReadsTheReport(t *testing.T) {
	body := liveReportBody(t)
	var hits int64
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/usage/report" {
			t.Errorf("the watcher requested %q — /usage/report is the only route this edge may build", r.URL.Path)
			w.WriteHeader(http.StatusNotFound)
			return
		}
		atomic.AddInt64(&hits, 1)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	w := newUsageWatcher(srv.URL)
	ctx := context.Background()

	// A watcher nobody has asked holds nothing and polls nothing.
	if w.sampling(time.Now()) {
		t.Error("the poller is running before anybody asked — this is the ambient cost the wind-down exists to remove")
	}

	snap, err := w.Latest(ctx)
	if err != nil {
		t.Fatalf("Latest: %v", err)
	}
	if atomic.LoadInt64(&hits) != 1 {
		t.Fatalf("the first ask made %d requests, want exactly 1 — an ask on a cold watcher must fetch inline rather than answer empty", hits)
	}
	if !w.sampling(time.Now()) {
		t.Error("asking must start the poller")
	}
	if snap.Empty() {
		t.Fatal("the fetched document decoded to no providers")
	}

	// The document arrived intact and the currency rule survived the round
	// trip: judged at its own generated_at, claude's default 5h window reads
	// 18% with 5596s left.
	buckets := snap.Buckets(time.Unix(1788126404, 0))
	var found bool
	for _, b := range buckets {
		if b.ID() != "claude//five_hour" {
			continue
		}
		found = true
		used, ok := b.Reading.UsedPercent()
		if !ok || used != 18 {
			t.Errorf("UsedPercent() = %v, %v; want 18", used, ok)
		}
	}
	if !found {
		t.Errorf("claude//five_hour not among %d buckets", len(buckets))
	}

	// The SECOND ask inside usageMaxAge is answered from the held document.
	if _, err := w.Latest(ctx); err != nil {
		t.Fatalf("second Latest: %v", err)
	}
	if got := atomic.LoadInt64(&hits); got != 1 {
		t.Errorf("a second ask inside usageMaxAge made %d requests total, want 1", got)
	}
}

// The poller must not run on a machine nobody has asked, and must stop once the
// asking stops. This is fleetWatcher's discipline and it is the only thing
// standing between this feature and an unconditional 30-second HTTP GET on
// every desktop install.
func TestUsageWatcherWindsDownWhenNobodyIsAsking(t *testing.T) {
	w := newUsageWatcher("http://127.0.0.1:1")
	now := time.Unix(1788126404, 0)

	if w.sampling(now) {
		t.Fatal("sampling before any ask")
	}
	w.noteAsk(now)
	if !w.sampling(now) {
		t.Error("not sampling immediately after an ask")
	}
	if !w.sampling(now.Add(usageSampleIdleAfter)) {
		t.Error("stopped sampling at exactly the idle horizon")
	}
	if w.sampling(now.Add(usageSampleIdleAfter + time.Second)) {
		t.Error("still sampling past the idle horizon — the poller never winds down")
	}
}

// A daemon that cannot be reached yields UNKNOWN, not the last good numbers.
// Keeping a stale document alive across an outage is the same defect the
// currency rule closes, one level up: the routing layer would keep deciding
// from figures nobody can vouch for.
func TestUsageWatcherDoesNotServeStaleNumbersAcrossAnOutage(t *testing.T) {
	body := liveReportBody(t)
	var up atomic.Bool
	up.Store(true)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !up.Load() {
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	w := newUsageWatcher(srv.URL)
	ctx := context.Background()
	if _, err := w.Latest(ctx); err != nil {
		t.Fatalf("first Latest: %v", err)
	}

	up.Store(false)
	// Age the held document past usageMaxAge so the next ask refetches.
	w.mu.Lock()
	w.snap.FetchedAt = time.Now().Add(-2 * usageMaxAge)
	w.mu.Unlock()

	if _, err := w.Latest(ctx); err == nil {
		t.Fatal("a 503 from the daemon answered with the previous document — 'the daemon is unreachable' must be UNKNOWN, not 'whatever it said last time'")
	}
}

func TestUsageWatcherRefusesABodyThatIsNotTheReport(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("<html>proxy error</html>"))
	}))
	defer srv.Close()

	if _, err := newUsageWatcher(srv.URL).Latest(context.Background()); err == nil {
		t.Fatal("a 200 carrying something other than the report must be an error: no answer and zero usage are different claims")
	}
}

// A guard against the one shape this file must never grow: a verdict computed
// once and held. The whole layer's correctness rests on Snapshot being judged
// against the DECIDING instant, so the watcher hands out the document and
// nothing else.
func TestTheWatcherHoldsTheDocumentUnjudged(t *testing.T) {
	body := liveReportBody(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	snap, err := newUsageWatcher(srv.URL).Latest(context.Background())
	if err != nil {
		t.Fatalf("Latest: %v", err)
	}
	var _ limits.Snapshot = snap

	at := func(offset time.Duration) (limits.Bucket, bool) {
		for _, b := range snap.Buckets(time.Unix(1788126404, 0).Add(offset)) {
			if b.ID() == "claude//five_hour" {
				return b, true
			}
		}
		return limits.Bucket{}, false
	}
	fresh, ok := at(0)
	if !ok || !fresh.Reading.Usable() {
		t.Fatal("at the document's own instant the window is running")
	}
	stale, ok := at(3 * time.Hour)
	if !ok {
		t.Fatal("bucket missing")
	}
	if stale.Reading.Usable() {
		t.Fatal("the SAME held document must yield UNKNOWN three hours later — a cached verdict is the defect")
	}
	if _, ok := stale.Reading.UsedPercent(); ok {
		t.Error("a percentage escaped off a window that has since closed")
	}
}

// A ROUTING DECISION TAKES ITS OWN READING.
//
// Latest's cache is right for a status pane and wrong for a decision: the
// currency guard stops a WINDOW verdict outliving its instant, and this stops
// the DOCUMENT outliving its instant, which is the same defect one level out. A
// window that reset thirty seconds ago is unreadable in a document fetched two
// minutes ago and readable in one fetched now, and a routing decision commits an
// hour of a frontier model's allowance on the difference.
func TestARoutingDecisionTakesItsOwnReading(t *testing.T) {
	body := liveReportBody(t)
	var hits int64
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		atomic.AddInt64(&hits, 1)
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	w := newUsageWatcher(srv.URL)
	ctx := context.Background()
	for i := 1; i <= 3; i++ {
		if _, err := w.LatestWithin(ctx, usageDecisionMaxAge); err != nil {
			t.Fatalf("decision %d: %v", i, err)
		}
		if got := atomic.LoadInt64(&hits); got != int64(i) {
			t.Fatalf("after %d decisions the daemon had been read %d time(s) — a decision that reuses a cached document is deciding against a picture that may have rolled over since", i, got)
		}
	}

	// And the ordinary bound is untouched: three back-to-back Latest calls after
	// those must add exactly one more fetch, not three.
	before := atomic.LoadInt64(&hits)
	for i := 0; i < 3; i++ {
		if _, err := w.Latest(ctx); err != nil {
			t.Fatalf("Latest: %v", err)
		}
	}
	if got := atomic.LoadInt64(&hits) - before; got > 1 {
		t.Errorf("three Latest calls inside usageMaxAge made %d fetches — the decision bound leaked into the ordinary one", got)
	}
}

// routing.select must answer even when nothing about the machine cooperates,
// because a hub that refuses to route while claudemon restarts is worse than one
// that routes conservatively and says why.
func TestRoutingSelectAnswersWithoutAUsageDocument(t *testing.T) {
	svc := routing.New("", nil) // compiled-in defaults, no file, no catalog
	h := routingSelect(svc, newUsageWatcher("http://127.0.0.1:1"))

	if _, err := h(bus.CallerIdentity{}, json.RawMessage(`{}`)); err == nil {
		t.Error("a request with no role was answered — routing answers in ROLES, and guessing one is how a decision gets attributed to work nobody described")
	}

	raw, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"scout","cwd":"/tmp"}`))
	if err != nil {
		t.Fatalf("routing.select: %v", err)
	}
	d, ok := raw.(routing.Decision)
	if !ok {
		t.Fatalf("handler returned %T", raw)
	}
	if !d.Eligible || d.Model == "" {
		t.Errorf("no model with the daemon unreachable: %+v", d)
	}
	if d.Capacity.Health != limits.HealthUnknown {
		t.Errorf("health = %q with the daemon unreachable, want unknown", d.Capacity.Health)
	}
	if d.Mode != routing.ModeNormal {
		t.Errorf("mode = %q on an unknown capacity — an unreadable provider is not a constrained one and is not a spendable one either", d.Mode)
	}
	if len(d.Reason) == 0 {
		t.Error("no reasons")
	}
}
