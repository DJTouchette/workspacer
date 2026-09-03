package routing

import (
	"bytes"
	"context"
	"errors"
	"log"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// errNoAnswer is "could not ask", which is the one catalog error that means
// nothing about whether the matrix is right.
var errNoAnswer = errors.New("nobody answered claude.listModels")

func countIssuesAt(issues []Issue, where string) int {
	n := 0
	for _, iss := range issues {
		if iss.Where == where {
			n++
		}
	}
	return n
}

// The boot deadlock these tests exist for:
//
// the hub's HTTP listener does not bind until routing.New has returned, and the
// catalog's claude half is answered over that very listener's bus by the desktop
// (which only connects once /health answers). A catalog probe inside New is
// therefore a question that cannot be answered until New has returned, and it
// cost the hub a full bus-client readiness window (5s) on EVERY boot, which is
// exactly the "control plane is slow to start" banner the desktop showed.
//
// So: New parses and installs the matrix and asks nothing, and the check runs on
// the tick, which is the first moment it can actually succeed.

// slowCatalog answers on demand and records who asked. release nil means "answer
// at once"; a non-nil release holds every answer until it is closed, which is
// how a probe on the boot path is made visible as a stall rather than a hang.
type slowCatalog struct {
	mu      sync.Mutex
	asked   []string
	release chan struct{}
	models  map[string][]CatalogModel
	errs    map[string]error
}

func (c *slowCatalog) Models(provider string) ([]CatalogModel, error) {
	c.mu.Lock()
	c.asked = append(c.asked, provider)
	c.mu.Unlock()
	if c.release != nil {
		<-c.release
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if err, ok := c.errs[provider]; ok {
		return nil, err
	}
	return c.models[provider], nil
}

func (c *slowCatalog) calls() []string {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]string(nil), c.asked...)
}

// syncBuffer is a log sink several goroutines may write to.
type syncBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

func captureLog(t *testing.T) *syncBuffer {
	t.Helper()
	var b syncBuffer
	flags, out := log.Flags(), log.Writer()
	log.SetOutput(&b)
	log.SetFlags(0)
	t.Cleanup(func() {
		log.SetOutput(out)
		log.SetFlags(flags)
	})
	return &b
}

// waitFor polls until cond or the deadline, so a tick-driven check is asserted
// on without a sleep the size of the tick.
func waitFor(t *testing.T, why string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(2 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", why)
}

// TestNewDoesNotWaitOnTheCatalog is the boot-latency regression test: nothing in
// New may block on a live probe, because the hub binds its listener after New
// returns and the probe cannot be answered before it binds.
func TestNewDoesNotWaitOnTheCatalog(t *testing.T) {
	path := matrixPath(t)
	cat := &slowCatalog{release: make(chan struct{})}
	defer close(cat.release)

	done := make(chan *Service, 1)
	go func() { done <- New(path, cat) }()

	select {
	case s := <-done:
		m := s.Matrix()
		if m == nil {
			t.Fatal("New returned with no matrix installed")
		}
		if m.CatalogChecked {
			t.Error("the matrix claims it was checked against the catalog, which New must not have done")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("New blocked on the model catalog: the hub cannot bind its listener until New returns, and the catalog cannot answer until it has")
	}

	if got := cat.calls(); len(got) != 0 {
		t.Errorf("New probed the catalog for %v; the boot path must ask nothing", got)
	}
}

// TestTheCatalogCheckRunsOnTheFirstTick: deferred is not dropped. The findings
// still arrive, they still ride Matrix.Issues, and they are still logged.
func TestTheCatalogCheckRunsOnTheFirstTick(t *testing.T) {
	logs := captureLog(t)
	path := matrixPath(t)
	write(t, path, `active_profile: codex_only
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-retired, effort: high }
`)
	cat := &slowCatalog{models: map[string][]CatalogModel{
		"codex": {{ID: "gpt-5.6-sol", EffortLevels: []string{"low", "medium", "high", "xhigh"}}},
	}}
	s := New(path, cat)
	if hasIssueAt(s.Matrix().Issues, "profiles.codex_only.frontier") {
		t.Fatal("the catalog was consulted at boot after all")
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go s.Run(ctx, 5*time.Millisecond)

	waitFor(t, "the first tick's catalog check", func() bool { return s.Matrix().CatalogChecked })
	m := s.Matrix()
	if !hasIssueAt(m.Issues, "profiles.codex_only.frontier") {
		t.Errorf("the deferred catalog check lost its finding: %v", m.Issues)
	}
	if !strings.Contains(logs.String(), "does not serve model") {
		t.Errorf("the finding was not logged where it always was:\n%s", logs.String())
	}
}

// TestTheCatalogVerdictReachesTheDecision: the telemetry must not read an
// unchecked matrix as a clean one, so a decision says whether the check has run.
func TestTheCatalogVerdictReachesTheDecision(t *testing.T) {
	path := matrixPath(t)
	cat := &slowCatalog{models: map[string][]CatalogModel{"codex": {{ID: "gpt-5.6-sol"}}}}
	s := New(path, cat)

	d := Select(s.Matrix(), limits.Snapshot{}, nil, nil, time.Now(), Request{Role: "implementer"})
	if d.Matrix.CatalogChecked {
		t.Error("a decision taken before the check claims the matrix was checked")
	}
	if !s.ValidateCatalog() {
		t.Fatal("ValidateCatalog did not run")
	}
	d = Select(s.Matrix(), limits.Snapshot{}, nil, nil, time.Now(), Request{Role: "implementer"})
	if !d.Matrix.CatalogChecked {
		t.Error("a decision taken after the check does not say so")
	}
}

// TestAReloadIsCheckedAgainstTheCatalogAgain: a hand edit gets its own check,
// on the same tick that picked it up, and the findings do not pile up.
func TestAReloadIsCheckedAgainstTheCatalogAgain(t *testing.T) {
	path := matrixPath(t)
	write(t, path, `active_profile: codex_only
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-sol, effort: high }
`)
	cat := &slowCatalog{models: map[string][]CatalogModel{
		"codex": {{ID: "gpt-5.6-sol", EffortLevels: []string{"low", "medium", "high", "xhigh"}}},
	}}
	s := New(path, cat)
	s.ValidateCatalog()
	if hasIssueAt(s.Matrix().Issues, "profiles.codex_only.frontier") {
		t.Fatalf("a model the catalog serves was flagged: %v", s.Matrix().Issues)
	}

	write(t, path, `active_profile: codex_only
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-retired, effort: high }
`)
	if !s.ReloadIfChanged() {
		t.Fatal("the edit was not picked up")
	}
	if s.Matrix().CatalogChecked {
		t.Error("a freshly reloaded document claims it was already checked")
	}
	if !s.ValidateCatalog() {
		t.Fatal("the reload did not re-arm the catalog check")
	}
	if n := countIssuesAt(s.Matrix().Issues, "profiles.codex_only.frontier"); n != 1 {
		t.Errorf("the edited row carries %d findings, want exactly 1: %v", n, s.Matrix().Issues)
	}
}

// TestAnUnansweredProviderIsAskedAgain: a provider that could not answer leaves
// the check pending, because a daemon coming back is the ordinary case; and the
// second pass must not double the findings it already has.
func TestAnUnansweredProviderIsAskedAgain(t *testing.T) {
	path := matrixPath(t)
	write(t, path, `active_profile: codex_only
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-retired, effort: high }
`)
	cat := &slowCatalog{
		models: map[string][]CatalogModel{"codex": {{ID: "gpt-5.6-sol"}}},
		errs:   map[string]error{"claude": errNoAnswer},
	}
	s := New(path, cat)
	s.retryEvery = 0 // the cadence is Run's business; this test is about the retry itself

	if !s.ValidateCatalog() {
		t.Fatal("the first check did not run")
	}
	if !s.CatalogPending() {
		t.Error("a provider that could not answer settled the check anyway")
	}
	if !s.ValidateCatalog() {
		t.Fatal("a pending check was not retried")
	}
	if n := countIssuesAt(s.Matrix().Issues, "profiles.codex_only.frontier"); n != 1 {
		t.Errorf("a retried check piled up %d copies of the same finding: %v", n, s.Matrix().Issues)
	}

	// The daemon comes back: the check settles and stops asking.
	cat.mu.Lock()
	cat.errs = map[string]error{}
	cat.models["claude"] = []CatalogModel{{ID: "opus"}, {ID: "sonnet"}, {ID: "fable"}, {ID: "haiku"}}
	cat.mu.Unlock()
	if !s.ValidateCatalog() {
		t.Fatal("the check was not retried once the provider was back")
	}
	if s.CatalogPending() {
		t.Error("every provider answered and the check still says it is pending")
	}
	if s.ValidateCatalog() {
		t.Error("a settled check kept probing the catalog on every tick")
	}
}

// TestTheFirstCatalogCheckDoesNotWaitForTheOrdinaryTick is SHOULD-FIX 1a: the
// fallover walk (alternatives.go's unusable) reads Matrix.Issues to judge a
// candidate, and between boot and the first DefaultTickEvery tick (30s) those
// catalog findings do not exist yet, so a catalog-invalid alternative reads as
// clean for that whole window. Run must fire its own short first check well
// ahead of the ordinary tick rather than waiting on it.
func TestTheFirstCatalogCheckDoesNotWaitForTheOrdinaryTick(t *testing.T) {
	path := matrixPath(t)
	cat := &slowCatalog{models: map[string][]CatalogModel{
		"codex":  {{ID: "gpt-5.6-sol"}},
		"claude": {{ID: "opus"}, {ID: "sonnet"}, {ID: "fable"}},
	}}
	s := New(path, cat)
	s.firstCheckDelay = 5 * time.Millisecond

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	// An hour-long ordinary tick: if the check ran on THAT rather than on
	// firstCheckDelay, this test would time out inside waitFor rather than
	// pass, which is the point — it proves the two are on separate timers.
	go s.Run(ctx, time.Hour)

	waitFor(t, "the short first check", func() bool { return s.Matrix().CatalogChecked })
}

// refreshTrackingCatalog is a Catalog that also implements RefreshingCatalog,
// counting each provider it is asked about on the two different doors
// separately — the only way to prove SHOULD-FIX 2: that a RETRY goes through
// Refresh rather than replaying whatever Models would have answered from its
// own cache.
type refreshTrackingCatalog struct {
	mu           sync.Mutex
	modelsAsked  []string
	refreshAsked []string
	errs         map[string]error
	models       map[string][]CatalogModel
}

func (c *refreshTrackingCatalog) Models(provider string) ([]CatalogModel, error) {
	c.mu.Lock()
	c.modelsAsked = append(c.modelsAsked, provider)
	err, hasErr := c.errs[provider]
	models := c.models[provider]
	c.mu.Unlock()
	if hasErr {
		return nil, err
	}
	return models, nil
}

func (c *refreshTrackingCatalog) Refresh(provider string) ([]CatalogModel, error) {
	c.mu.Lock()
	c.refreshAsked = append(c.refreshAsked, provider)
	err, hasErr := c.errs[provider]
	models := c.models[provider]
	c.mu.Unlock()
	if hasErr {
		return nil, err
	}
	return models, nil
}

// TestARetryForcesAFreshProbeRatherThanReadingTheCache is SHOULD-FIX 2.
//
// cmd/hub's routingCatalog caches a provider's answer for catalogTTL (10m),
// longer than DefaultCatalogRetryEvery (5m). A retry that went through Models
// — the same door an ordinary check uses — would keep reading the very cached
// miss it exists to get past, so the first REAL re-probe would land at
// catalogTTL rather than at the retry cadence the service asked for. The fix
// is Service.ValidateCatalog calling RefreshingCatalog.Refresh on a retry
// instead, and this is what proves it actually happens rather than merely
// compiling.
func TestARetryForcesAFreshProbeRatherThanReadingTheCache(t *testing.T) {
	path := matrixPath(t)
	write(t, path, `active_profile: codex_only
profiles:
  codex_only:
    frontier: { provider: codex, model: gpt-5.6-retired, effort: high }
`)
	cat := &refreshTrackingCatalog{
		errs:   map[string]error{"claude": errNoAnswer}, // unanswered -> the check stays pending
		models: map[string][]CatalogModel{"codex": {{ID: "gpt-5.6-sol"}}},
	}
	s := New(path, cat)
	s.retryEvery = 0 // the cadence is Run's business; this test is about the retry itself

	if !s.ValidateCatalog() {
		t.Fatal("the first check did not run")
	}
	if len(cat.refreshAsked) != 0 {
		t.Errorf("the FIRST check forced a refresh rather than an ordinary probe: %v", cat.refreshAsked)
	}
	if !s.CatalogPending() {
		t.Fatal("fixture drift: the unanswered provider must leave the check pending, or this test proves nothing")
	}

	if !s.ValidateCatalog() {
		t.Fatal("the retry did not run")
	}
	if len(cat.refreshAsked) == 0 {
		t.Fatal("the retry read the cache instead of forcing a fresh probe — the exact no-op SHOULD-FIX 2 closes")
	}
}
