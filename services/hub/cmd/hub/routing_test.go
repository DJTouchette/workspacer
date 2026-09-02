package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/event"
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
	h := routingSelect(svc, newUsageWatcher("http://127.0.0.1:1"), nil,
		nil, nil)

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

// ---------------------------------------------------------------------------
// THE STAMPEDE
// ---------------------------------------------------------------------------

// TestConcurrentDecisionsShareOneFetch is the singleflight.
//
// Every decision takes its own reading (usageDecisionMaxAge is 0), which was
// affordable while routing was advisory and rarely asked. Once dispatch routes
// through it, N concurrent decisions were N concurrent GETs against one loopback
// daemon, each queued behind the others' work — and the fix must NOT be a
// time-based cache, because a cached document is the founding defect of this
// whole feature. Sharing one OPEN request is fine; serving a CLOSED one is not,
// and the second half of this test is what tells the two apart.
func TestConcurrentDecisionsShareOneFetch(t *testing.T) {
	body := liveReportBody(t)
	var hits int64
	release := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		atomic.AddInt64(&hits, 1)
		<-release // hold every request open until all the callers have arrived
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	w := newUsageWatcher(srv.URL)
	const callers = 8
	done := make(chan error, callers)
	for i := 0; i < callers; i++ {
		go func() {
			_, err := w.LatestWithin(context.Background(), usageDecisionMaxAge)
			done <- err
		}()
	}
	// Wait until the first request is actually in the handler, then give the
	// others time to pile onto it rather than starting their own.
	deadline := time.Now().Add(3 * time.Second)
	for atomic.LoadInt64(&hits) == 0 && time.Now().Before(deadline) {
		time.Sleep(2 * time.Millisecond)
	}
	time.Sleep(150 * time.Millisecond)
	if got := atomic.LoadInt64(&hits); got != 1 {
		close(release)
		t.Fatalf("%d concurrent decisions produced %d requests against claudemon — every spawn now routes through this, so a stampede is the ordinary case rather than the exotic one", callers, got)
	}
	close(release)
	for i := 0; i < callers; i++ {
		if err := <-done; err != nil {
			t.Fatalf("shared fetch failed for one of the waiters: %v", err)
		}
	}

	// AND IT IS NOT A CACHE. The next decision, arriving after the shared fetch
	// has landed, must take its OWN reading — a document that is correct at fetch
	// time and read at some later, unrelated instant is exactly what this layer
	// exists to close.
	before := atomic.LoadInt64(&hits)
	if _, err := w.LatestWithin(context.Background(), usageDecisionMaxAge); err != nil {
		t.Fatalf("decision after the shared fetch: %v", err)
	}
	if got := atomic.LoadInt64(&hits) - before; got != 1 {
		t.Errorf("a decision arriving after the shared fetch made %d requests, want 1 — the singleflight turned into a cache, which is the defect it was supposed to avoid", got)
	}
}

// TestASlowClaudemonCannotHangADecision: a hub whose usage daemon stops
// answering must degrade the ANSWER, never stall the fleet. Routing is on the
// dispatch path now, so "the decision waits as long as the HTTP client would" is
// not good enough — a hung connect can sit there for the whole probe timeout.
//
// What a decision does with no reading is the deliberate part: it routes as if
// capacity were UNKNOWN and says so. Not a refusal (a hub that would not
// dispatch because a usage daemon was restarting is worse than the advisory
// layer it replaced) and not a healthy default (UNKNOWN goes through the
// matrix's own when_unknown and satisfies neither mode arm, so the answer is
// NORMAL — no promotion off evidence that does not exist, no phantom
// conservation).
func TestASlowClaudemonCannotHangADecision(t *testing.T) {
	block := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		select {
		case <-block:
		case <-r.Context().Done():
		}
	}))
	// Released BEFORE the server is torn down: Close waits for in-flight
	// handlers, and the detached fetch this test abandons is still one.
	defer func() { close(block); srv.Close() }()

	svc := routing.New("", nil)
	h := routingSelect(svc, newUsageWatcher(srv.URL), nil,
		nil, nil)

	// The handler's own budget, shortened for the test by bounding the wall
	// clock rather than the constant: what is being proven is that SOME bound
	// exists and that the answer past it is a routed decision, not an error.
	start := time.Now()
	raw, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"scout","cwd":"/tmp"}`))
	elapsed := time.Since(start)
	if err != nil {
		t.Fatalf("a hanging daemon turned into a refused routing call: %v", err)
	}
	if elapsed >= usageProbeTimeout {
		t.Errorf("the decision waited %s — it must be bounded by usageDecisionWait (%s), not by the HTTP probe timeout (%s)",
			elapsed.Round(time.Millisecond), usageDecisionWait, usageProbeTimeout)
	}
	d, ok := raw.(routing.Decision)
	if !ok {
		t.Fatalf("handler returned %T", raw)
	}
	if !d.Eligible || d.Model == "" {
		t.Errorf("no model came back from a decision made with no reading: %+v", d)
	}
	if d.Capacity.Health != limits.HealthUnknown {
		t.Errorf("health = %q against a daemon that never answered, want unknown", d.Capacity.Health)
	}
	if d.Mode != routing.ModeNormal {
		t.Errorf("mode = %q with no reading at all, want normal — neither a promotion nor a phantom conservation is licensed by evidence that does not exist", d.Mode)
	}
}

// The decision the handler answers is also the decision it RECORDS and
// PUBLISHES, joined by one id. Without this the decisionId on the spawn wire
// would be a field nothing on the other end ever wrote.
func TestRoutingSelectRecordsAndPublishesTheDecisionItAnswered(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	logf := routing.NewDecisionLog(path, routing.DefaultDecisionLogMaxBytes)

	var published []event.Envelope
	h := routingSelect(routing.New("", nil), newUsageWatcher("http://127.0.0.1:1"), nil,
		func(ev event.Envelope) { published = append(published, ev) }, logf)

	raw, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"implementer","cwd":"/tmp","provider":"codex"}`))
	if err != nil {
		t.Fatalf("routing.select: %v", err)
	}
	d := raw.(routing.Decision)
	if d.DecisionID == "" {
		t.Fatal("the answer carries no decisionId, so no spawn can ever quote it")
	}

	if len(published) != 1 || published[0].Type != routingDecisionTopic {
		t.Fatalf("published %d event(s): %+v", len(published), published)
	}
	var ev map[string]any
	if err := json.Unmarshal(published[0].Data, &ev); err != nil {
		t.Fatalf("event payload: %v", err)
	}
	if ev["decisionId"] != d.DecisionID {
		t.Errorf("the event's decisionId %v does not match the answer's %q", ev["decisionId"], d.DecisionID)
	}
	if ev["model"] != d.Model || ev["mode"] != string(d.Mode) {
		t.Errorf("the published projection disagrees with the answer: %v vs %+v", ev, d)
	}
	// The cwd is deliberately NOT on an open-by-decision topic.
	if _, present := ev["cwd"]; present {
		t.Errorf("the published event carries the caller's cwd, which every tier receives: %v", ev)
	}

	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("the decision log was not written: %v", err)
	}
	if !strings.Contains(string(body), d.DecisionID) {
		t.Errorf("the log does not carry the answered decision's id:\n%s", body)
	}
	if !strings.Contains(string(body), `"kind":"decision"`) {
		t.Errorf("the log row is not a decision row:\n%s", body)
	}
}

// ---------------------------------------------------------------------------
// PER-CAPABILITY ALTERNATIVES, AT THE EVENT BOUNDARY
// ---------------------------------------------------------------------------

// altUsageDoc builds a minimal /usage/report body naming exactly the
// providers given, each a single default account with a five_hour + seven_day
// reading — the same two-window shape internal/routing's own policy_test.go
// altGreen/altRed use, chosen so RED/GREEN fold the same way here. `used` is
// the five_hour window's used_percent; seven_day is fixed comfortably healthy
// so it never becomes the binding window.
func altUsageDoc(t *testing.T, now time.Time, byProvider map[string]float64) []byte {
	t.Helper()
	window := func(used float64, resets time.Duration) map[string]any {
		return map[string]any{
			"window_minutes": nil,
			"is_current":     nil,
			"used_percent":   map[string]any{"state": "ok", "value": used},
			"resets_at":      now.Add(resets).Unix(),
		}
	}
	names := make([]string, 0, len(byProvider))
	for name := range byProvider {
		names = append(names, name)
	}
	sort.Strings(names)
	providers := make([]any, 0, len(names))
	for _, name := range names {
		providers = append(providers, map[string]any{
			"provider": name,
			"accounts": []any{map[string]any{
				"account": "", "label": "test", "is_default": true,
				"source": "oauth_poll",
				"windows": map[string]any{
					"five_hour": window(byProvider[name], 4*time.Hour),
					"seven_day": window(11, 96*time.Hour),
					"monthly": map[string]any{
						"window_minutes": nil, "is_current": nil,
						"used_percent": map[string]any{"state": "unavailable", "reason": "test fixture carries no monthly window"},
						"resets_at":    nil,
					},
				},
			}},
		})
	}
	raw, err := json.Marshal(map[string]any{"generated_at": now.Unix(), "providers": providers})
	if err != nil {
		t.Fatalf("marshal usage doc: %v", err)
	}
	return raw
}

// TestAFalloverEventNamesTheHealthOfTheProviderItActuallyPicked is SHOULD-FIX
// 2, proved at the RUNTIME boundary this handler owns.
//
// Before the fix, the published routing.decision event's `health` field
// always quoted d.Capacity.Health — the SUBJECT's reading (step 4), never
// revisited after a fallover — so a reviewer that fell over from a red claude
// to a green codex shipped `provider: codex, health: red` onto the
// open-by-decision event plane: the primary's health, misattributed to the
// provider actually running the work. Decision.EffectiveCapacity is the fix;
// this proves it through routingSelect, the handler that actually builds the
// event, rather than only inside the pure policy layer.
func TestAFalloverEventNamesTheHealthOfTheProviderItActuallyPicked(t *testing.T) {
	now := time.Now()
	body := altUsageDoc(t, now, map[string]float64{"claude": 95, "codex": 12})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	logf := routing.NewDecisionLog(filepath.Join(t.TempDir(), "routing-decisions.jsonl"), routing.DefaultDecisionLogMaxBytes)
	var published []event.Envelope
	h := routingSelect(routing.New("", nil), newUsageWatcher(srv.URL), nil,
		func(ev event.Envelope) { published = append(published, ev) }, logf)

	raw, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"reviewer","cwd":"/tmp"}`))
	if err != nil {
		t.Fatalf("routing.select: %v", err)
	}
	d, ok := raw.(routing.Decision)
	if !ok {
		t.Fatalf("handler returned %T", raw)
	}
	if !d.Eligible || d.Provider != "codex" {
		t.Fatalf("got %+v, want a fallover onto codex — the rest of this test proves nothing otherwise", d)
	}
	if d.Capacity.Health != limits.HealthRed {
		t.Fatalf("d.Capacity.Health = %q, want claude's RED reading — this test needs the primary to actually be unusable to mean anything", d.Capacity.Health)
	}

	if len(published) != 1 {
		t.Fatalf("published %d event(s): %+v", len(published), published)
	}
	var ev map[string]any
	if err := json.Unmarshal(published[0].Data, &ev); err != nil {
		t.Fatalf("event payload: %v", err)
	}
	if ev["provider"] != "codex" {
		t.Fatalf("event provider = %v, want codex", ev["provider"])
	}
	if ev["health"] != string(limits.HealthGreen) {
		t.Errorf("event health = %v, want green — codex's OWN reading, agreeing with the provider the event names, not claude's red", ev["health"])
	}
}

// TestAFalloverDecisionPublishesFellOverFrom is minor fix (b): the event's
// FellOverFrom wiring in routingSelect is one unguarded field assignment with
// no test that would notice its removal — deleting it leaves cmd/hub green.
// Without it, a client watching the open-by-decision event plane sees a
// reviewer land on the implementer's own family with no explanation in the
// payload at all; the reason sentence explaining why exists only in the
// answer the caller of routing.select gets directly, not on the wire every
// tier receives.
func TestAFalloverDecisionPublishesFellOverFrom(t *testing.T) {
	now := time.Now()
	body := altUsageDoc(t, now, map[string]float64{"claude": 95, "codex": 12})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	logf := routing.NewDecisionLog(filepath.Join(t.TempDir(), "routing-decisions.jsonl"), routing.DefaultDecisionLogMaxBytes)
	var published []event.Envelope
	h := routingSelect(routing.New("", nil), newUsageWatcher(srv.URL), nil,
		func(ev event.Envelope) { published = append(published, ev) }, logf)

	if _, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"reviewer","cwd":"/tmp"}`)); err != nil {
		t.Fatalf("routing.select: %v", err)
	}
	if len(published) != 1 {
		t.Fatalf("published %d event(s): %+v", len(published), published)
	}
	var ev struct {
		FellOverFrom *routing.Assignment `json:"fellOverFrom"`
	}
	if err := json.Unmarshal(published[0].Data, &ev); err != nil {
		t.Fatalf("event payload: %v", err)
	}
	if ev.FellOverFrom == nil || ev.FellOverFrom.Provider != "claude" {
		t.Fatalf("event fellOverFrom = %+v, want the claude primary this decision fell over from", ev.FellOverFrom)
	}
}

// stubAvailability is a routingCatalog stand-in: it answers with a fixed map
// and records what the handler asked it to refresh.
type stubAvailability struct {
	live      routing.ProviderAvailability
	refreshed [][]string
}

func (s *stubAvailability) Availability() routing.ProviderAvailability { return s.live }
func (s *stubAvailability) RefreshAvailability(providers []string) {
	s.refreshed = append(s.refreshed, providers)
}

// TestTheHandlerFeedsLiveAvailabilityIntoTheDecision is the wiring proof for
// slice 2's third part, at the boundary that owns it.
//
// routing.Select is pure and cannot probe anything, so the live launchability
// reading only exists if this handler reads it off the catalog and passes it
// in. A map that is built and never handed over would look exactly like a
// working feature from inside internal/routing, where every test supplies its
// own map by hand.
func TestTheHandlerFeedsLiveAvailabilityIntoTheDecision(t *testing.T) {
	now := time.Now()
	// BOTH providers healthy: nothing but availability can move this answer.
	body := altUsageDoc(t, now, map[string]float64{"claude": 12, "codex": 12})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	avail := &stubAvailability{live: routing.ProviderAvailability{
		"codex": {Available: false, Reason: "no codex CLI is installed on this machine"},
	}}
	logf := routing.NewDecisionLog(filepath.Join(t.TempDir(), "routing-decisions.jsonl"), routing.DefaultDecisionLogMaxBytes)
	h := routingSelect(routing.New("", nil), newUsageWatcher(srv.URL), avail, nil, logf)

	raw, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"implementer","cwd":"/tmp"}`))
	if err != nil {
		t.Fatalf("routing.select: %v", err)
	}
	d, ok := raw.(routing.Decision)
	if !ok {
		t.Fatalf("handler returned %T", raw)
	}
	if !d.Eligible || d.Provider != "claude" {
		t.Fatalf("got %s %s — the answer must fall over off the unavailable codex primary: %v", d.Provider, d.Model, d.Reason)
	}
	if !strings.Contains(strings.Join(d.Reason, " "), "no codex CLI is installed on this machine") {
		t.Errorf("the probe's own reason never reached the answer: %v", d.Reason)
	}

	// And the refresh is kicked for the providers this matrix can route to,
	// which is what keeps the map current without a background poll.
	if len(avail.refreshed) != 1 {
		t.Fatalf("the handler refreshed %d time(s); a decision must ask for a fresh reading exactly once", len(avail.refreshed))
	}
	got := strings.Join(avail.refreshed[0], ",")
	if !strings.Contains(got, "claude") || !strings.Contains(got, "codex") {
		t.Errorf("refreshed %q, want the providers the shipped matrix actually routes to", got)
	}

	// A handler with NO availability source at all is the ordinary desktop
	// case, and it must route exactly as it did before this existed.
	plain := routingSelect(routing.New("", nil), newUsageWatcher(srv.URL), nil, nil, logf)
	raw, err = plain(bus.CallerIdentity{}, json.RawMessage(`{"role":"implementer","cwd":"/tmp"}`))
	if err != nil {
		t.Fatalf("routing.select without a catalog: %v", err)
	}
	if d, _ := raw.(routing.Decision); d.Provider != "codex" {
		t.Errorf("provider = %q with no availability source — an unknown provider must fail OPEN", d.Provider)
	}
}

// TestTheDecisionEventCarriesTheEffortStep is the wiring proof for slice 2's
// first part on the OPEN-BY-DECISION event plane: without it a client watching
// the plane sees `effort` change with nothing in the payload saying a routing
// mode moved it.
func TestTheDecisionEventCarriesTheEffortStep(t *testing.T) {
	now := time.Now()
	// codex nearly out, claude fine. The request PINS codex, so the answer
	// stays there and the effort step is the only thing that can move.
	body := altUsageDoc(t, now, map[string]float64{"claude": 12, "codex": 95})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	}))
	defer srv.Close()

	logf := routing.NewDecisionLog(filepath.Join(t.TempDir(), "routing-decisions.jsonl"), routing.DefaultDecisionLogMaxBytes)
	var published []event.Envelope
	h := routingSelect(routing.New("", nil), newUsageWatcher(srv.URL), nil,
		func(ev event.Envelope) { published = append(published, ev) }, logf)

	raw, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"implementer","provider":"codex","cwd":"/tmp"}`))
	if err != nil {
		t.Fatalf("routing.select: %v", err)
	}
	d, _ := raw.(routing.Decision)
	if d.Mode != routing.ModeConserve || d.Effort != "medium" {
		t.Fatalf("got mode %s effort %q — this test needs a conserving codex to mean anything: %v", d.Mode, d.Effort, d.Reason)
	}
	if len(published) != 1 {
		t.Fatalf("published %d event(s)", len(published))
	}
	var ev struct {
		Effort     string              `json:"effort"`
		EffortStep *routing.EffortStep `json:"effortStep"`
	}
	if err := json.Unmarshal(published[0].Data, &ev); err != nil {
		t.Fatalf("event payload: %v", err)
	}
	if ev.Effort != "medium" {
		t.Errorf("event effort = %q, want the stepped `medium` the answer carries", ev.Effort)
	}
	if ev.EffortStep == nil || ev.EffortStep.From != "high" || ev.EffortStep.To != "medium" {
		t.Fatalf("event effortStep = %+v, want high -> medium", ev.EffortStep)
	}
	if ev.EffortStep.Why == "" {
		t.Error("the event carries a step with no explanation, so a display can say WHAT moved and never WHY")
	}

	// The other side: an ordinary answer no step was armed for must publish no
	// such field at all.
	quiet := altUsageDoc(t, now, map[string]float64{"claude": 12, "codex": 12})
	calm := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(quiet)
	}))
	defer calm.Close()
	published = nil
	h = routingSelect(routing.New("", nil), newUsageWatcher(calm.URL), nil,
		func(ev event.Envelope) { published = append(published, ev) }, logf)
	if _, err := h(bus.CallerIdentity{}, json.RawMessage(`{"role":"implementer","provider":"codex","cwd":"/tmp"}`)); err != nil {
		t.Fatalf("routing.select: %v", err)
	}
	if len(published) != 1 {
		t.Fatalf("published %d event(s)", len(published))
	}
	if strings.Contains(string(published[0].Data), "effortStep") {
		t.Errorf("a decision no step was armed for published one anyway: %s", published[0].Data)
	}
}

// TestAForcedRefreshReProbesAnUnavailableProvider is SHOULD-FIX 5.
//
// The three cached states are not equally worth holding. An answer with models
// in it describes a working install and is reused for catalogTTL. The other two
// are states somebody is expected to fix while the hub is running: a daemon that
// was down comes back, and a CLI that launched nothing gets installed or logged
// into. Holding "unavailable" for the full ten minutes made an installed
// provider unroutable for ten minutes after it started working, which is the one
// window in which a stale verdict costs the most.
func TestAForcedRefreshReProbesAnUnavailableProvider(t *testing.T) {
	var mu sync.Mutex
	var probes int
	serve := []string{} // what the fake claudemon says codex can launch

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !strings.HasSuffix(r.URL.Path, "/models") {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		mu.Lock()
		probes++
		models := make([]map[string]any, 0, len(serve))
		for _, id := range serve {
			models = append(models, map[string]any{"id": id})
		}
		mu.Unlock()
		_ = json.NewEncoder(w).Encode(map[string]any{"models": models})
	}))
	defer srv.Close()

	c := newRoutingCatalog(srv.URL, nil)

	// 1. The CLI runs and reports nothing: unavailable, with the reason.
	c.entry("codex", true)
	live := c.Availability()["codex"]
	if live.Available {
		t.Fatalf("codex = %+v, want the unavailable verdict a zero-model answer earns", live)
	}
	if !strings.Contains(live.Reason, "reported no launchable model") {
		t.Errorf("reason = %q, want the sentence naming what the probe actually saw", live.Reason)
	}
	if strings.Contains(live.Reason, "not installed") {
		t.Errorf("reason = %q claims a missing CLI, which this probe cannot detect: a missing binary makes the spawn fail, which is a non-2xx, which is UNKNOWN", live.Reason)
	}

	// 2. Somebody installs it. A FORCED refresh, well inside catalogTTL, has to
	//    ask again — that is the whole fix.
	mu.Lock()
	serve = []string{"gpt-5.6-sol"}
	before := probes
	mu.Unlock()

	c.entry("codex", true)
	mu.Lock()
	after := probes
	mu.Unlock()
	if after == before {
		t.Fatal("a forced refresh reused the cached `unavailable` verdict: a provider somebody just installed is exactly the one whose state changed")
	}
	if live := c.Availability()["codex"]; !live.Available {
		t.Fatalf("codex = %+v, want it routable again once its CLI answers with a model", live)
	}

	// 3. And a settled answer is still cached for the TTL, so the fix does not
	//    turn every decision back into a CLI boot.
	mu.Lock()
	before = probes
	mu.Unlock()
	c.entry("codex", true)
	c.entry("codex", true)
	mu.Lock()
	after = probes
	mu.Unlock()
	if after != before {
		t.Errorf("a provider answering WITH models was re-probed %d extra time(s) inside catalogTTL", after-before)
	}
}

// TestAnUnreachableDaemonAndAFailedProbeAreDifferentSentences keeps the two
// UNKNOWN shapes apart in the words while keeping them identical in the
// behaviour: both fail open, and neither ever reaches the availability map.
func TestAnUnreachableDaemonAndAFailedProbeAreDifferentSentences(t *testing.T) {
	// The daemon is up and answers 502 for this provider, which is the shape a
	// missing CLI makes: claudemon tries to spawn the binary and the spawn fails.
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusBadGateway)
	}))
	defer up.Close()

	c := newRoutingCatalog(up.URL, nil)
	if _, err := c.Models("codex"); err == nil || !strings.Contains(err.Error(), "502") {
		t.Errorf("error = %v, want the daemon's own status quoted so an operator can tell this from an unreachable daemon", err)
	}
	if live, ok := c.Availability()["codex"]; ok {
		t.Errorf("a failed probe produced an availability entry (%+v); it is UNKNOWN and must fail open", live)
	}

	// And a daemon that is not there at all.
	down := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	url := down.URL
	down.Close()

	c = newRoutingCatalog(url, nil)
	_, err := c.Models("codex")
	if err == nil || !strings.Contains(err.Error(), "could not be reached") {
		t.Errorf("error = %v, want the unreachable daemon said out loud", err)
	}
	if live, ok := c.Availability()["codex"]; ok {
		t.Errorf("an unreachable daemon produced an availability entry (%+v)", live)
	}
}
