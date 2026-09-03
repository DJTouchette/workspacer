package routing

import (
	"context"
	"crypto/sha256"
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// DefaultTickEvery is how often the matrix file is looked at. Same order as the
// job scheduler's poll, and for the same reason: hashing a few kilobytes on a
// half-minute tick costs nothing, and it is what makes hand-editing work.
const DefaultTickEvery = 30 * time.Second

// DefaultPath is where the routing matrix lives:
// <user-config-dir>/workspacer-hub/routing.yaml, falling back to the working
// directory when there is no config dir to anchor on — the same shape, and the
// same directory, as the hub's jobs.json.
func DefaultPath() string {
	dir, err := os.UserConfigDir()
	if err != nil || dir == "" {
		return "routing.yaml"
	}
	return filepath.Join(dir, "workspacer-hub", "routing.yaml")
}

// seedMarkerFor is the "we have offered this file once" record.
//
// Without it, deleting routing.yaml would put it straight back on the next boot,
// which is not what deleting a file means. libraryService.seedGlobalStarters
// settled the discipline: seed once, by id, so a starter deleted on purpose
// stays deleted and a starter added in a later release still arrives. Here there
// is one file, so the marker is one file too.
func seedMarkerFor(path string) string { return path + ".seeded" }

// Service holds the live matrix, seeds the file on first run, and re-reads it
// whenever its CONTENTS change.
//
// Change detection compares content rather than mtime, exactly as the job
// scheduler does: mtime granularity genuinely misses changes (a same-size write
// landing inside one timestamp tick), the file is small, and hashing what we
// wrote is also how the service tells its own seed write apart from a person's
// edit.
type Service struct {
	mu     sync.Mutex
	path   string
	cat    Catalog
	matrix *Matrix

	// specHash fingerprints the file's bytes as this service last saw them.
	specHash [sha256.Size]byte
	// haveSpecHash stays false until the file has been read or written once, so
	// the first look at an existing file is not mistaken for "unchanged".
	haveSpecHash bool
	// readErrLogged keeps an unreadable file from logging on every tick; it
	// resets as soon as a read succeeds.
	readErrLogged bool

	// baseIssues are the PURE load-time findings for the live matrix, kept apart
	// from the catalog's so a repeated catalog check rebuilds Matrix.Issues
	// rather than piling a second copy of the same finding onto it.
	baseIssues []Issue
	// catalogIssues is the last catalog check's verdict, kept so a retry that
	// says the same thing does not say it again on every tick.
	catalogIssues []Issue
	// catalogPending is "the live matrix still owes the catalog a check": true
	// from the moment a matrix is installed until a check runs in which every
	// provider it names actually answered.
	catalogPending bool
	// catalogChecking is the in-flight guard, because the check itself runs with
	// the lock RELEASED (see ValidateCatalog).
	catalogChecking bool
	catalogTriedAt  time.Time
	haveCatalogTry  bool
	// retryEvery bounds how often a pending check is retried. A field rather
	// than a constant so a test can drive the retry without waiting on it.
	retryEvery time.Duration
	// firstCheckDelay is how long after Run starts the FIRST catalog check
	// fires, ahead of the ordinary tick. A field, for the same reason
	// retryEvery is: a test drives it directly rather than waiting out
	// DefaultFirstCatalogCheckDelay for real.
	firstCheckDelay time.Duration
}

// New builds the service: seed the shipped default if this machine has never
// been offered it, then load.
//
// path "" disables the file entirely and runs on the compiled-in defaults, which
// is what tests and a read-only deployment want. cat may be nil, in which case
// model ids go unvalidated and that is said out loud once.
//
// NEW ASKS THE CATALOG NOTHING, AND THAT IS THE POINT. The hub does not bind its
// HTTP listener until this returns, and the catalog's claude half is answered
// over that listener's own bus by a desktop that only connects once /health
// answers, so a probe here is a question that cannot be answered until the
// question has been given up on. It was not merely useless: it cost the whole
// bus-client readiness window (5s) on EVERY boot, which is the "control plane is
// slow to start" banner the desktop showed. The check runs on the tick instead
// (see ValidateCatalog and Run), which is the first moment it can succeed.
func New(path string, cat Catalog) *Service {
	s := &Service{path: path, cat: cat, retryEvery: DefaultCatalogRetryEvery, firstCheckDelay: DefaultFirstCatalogCheckDelay}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.seedLocked()
	// Boot is just the first reload: same read, same parse, same merge, one code
	// path.
	if !s.reloadIfChangedLocked() && s.matrix == nil {
		// No file, or a file that could not be read: run on the defaults.
		if m, err := Defaults(); err == nil {
			m.fallback, _ = Defaults()
			s.installLocked(m)
			log.Printf("[routing] running on the compiled-in default matrix (no usable %s)", s.describePath())
		} else {
			log.Printf("[routing] the compiled-in default matrix does not load: %v", err)
		}
	}
	return s
}

// installLocked makes m the live matrix and re-arms the catalog check: a
// document nobody has checked yet is exactly what a freshly installed one is,
// whether it arrived at boot or from a hand edit ten minutes in.
func (s *Service) installLocked(m *Matrix) {
	s.matrix = m
	s.baseIssues = append([]Issue(nil), m.Issues...)
	s.catalogIssues = nil
	s.catalogPending = true
	s.haveCatalogTry = false
}

func (s *Service) describePath() string {
	if s.path == "" {
		return "matrix file (disabled)"
	}
	return s.path
}

// Path is the file this service watches ("" when disabled).
func (s *Service) Path() string { return s.path }

// Matrix is the matrix in force. Never nil once New has returned successfully;
// callers still get a nil-safe read because a matrix that failed to load at all
// is a bug worth surfacing rather than a panic worth hiding.
func (s *Service) Matrix() *Matrix {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.matrix
}

// ReloadIfChanged re-reads the file when its contents differ from what this
// service last saw, and reports whether the live matrix changed.
//
// This is the whole hand-editing story. Open routing.yaml in an editor, save it,
// and the next tick picks it up: a retuned threshold applies, a switched profile
// applies, an added ceiling applies. No restart, and no fsnotify either — the
// hub module is deliberately zero-dependency and polling is the house answer, so
// the poll rides a tick that was already there.
//
// The failure policy matters more than the happy path: a file that cannot be
// read, or that does not parse, leaves the running matrix EXACTLY as it was and
// logs. A half-typed edit, an editor unlinking the file for a moment during its
// own atomic save, or a backup tool moving it must not silently disarm routing —
// and because the merge is over compiled-in defaults, even an EMPTY document
// leaves every role answerable.
func (s *Service) ReloadIfChanged() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.reloadIfChangedLocked()
}

func (s *Service) reloadIfChangedLocked() bool {
	if s.path == "" {
		return false
	}
	raw, err := os.ReadFile(s.path)
	if err != nil {
		if !s.readErrLogged {
			s.readErrLogged = true
			if s.matrix != nil {
				log.Printf("[routing] %s cannot be read — keeping the matrix already loaded: %v", s.path, err)
			}
		}
		return false
	}
	s.readErrLogged = false
	sum := sha256.Sum256(raw)
	if s.haveSpecHash && sum == s.specHash {
		return false
	}
	// Recorded BEFORE the parse, so a file that is broken the same way every
	// tick is complained about once rather than twice a minute. Any further edit
	// changes the bytes and therefore gets another look.
	s.specHash, s.haveSpecHash = sum, true

	m, err := Load(s.path, raw)
	if err != nil {
		if s.matrix == nil {
			log.Printf("[routing] %s does not parse, running on the compiled-in defaults: %v", s.path, err)
		} else {
			log.Printf("[routing] %s does not parse — keeping the matrix already loaded: %v", s.path, err)
		}
		return false
	}
	s.installLocked(m)
	s.report(m)
	return true
}

// report is the load-time log, and it is what stops a matrix from half-applying
// in silence: the keys that MOVED something are named, one by one, and so is
// every key that matches nothing in the shipped defaults — which is the only
// place anyone finds out that the block they edited sat under a misspelled
// parent. The full set of keys the file carries is counted rather than
// enumerated; see keyPaths for why, and it stays on the Matrix for a caller that
// wants it.
func (s *Service) report(m *Matrix) {
	profile, fellBack := m.ActiveProfileName()
	where := m.Source
	if where == "" {
		where = "the compiled-in defaults"
	}
	log.Printf("[routing] loaded %s: version %d, profile %q%s, %d role(s), %d profile(s), %d ceiling(s)",
		where, m.Version, profile, map[bool]string{true: " (fell back — the file names one that does not exist)"}[fellBack],
		len(m.Roles), len(m.Profiles), len(m.Ceilings))
	if len(m.Applied) > 0 {
		if len(m.Changed) == 0 {
			log.Printf("[routing] %d key(s) taken from %s, none of which differ from the shipped defaults",
				len(m.Applied), m.Source)
		} else {
			log.Printf("[routing] %d key(s) taken from %s, %d of which change a shipped default: %s",
				len(m.Applied), m.Source, len(m.Changed), strings.Join(m.Changed, ", "))
		}
	}
	if len(m.Unrecognized) > 0 {
		log.Printf("[routing] %d key(s) in %s match nothing in the shipped defaults (usually a typo, still merged): %s",
			len(m.Unrecognized), m.Source, strings.Join(m.Unrecognized, ", "))
	}
	for _, iss := range m.Issues {
		log.Printf("[routing] %s", iss)
	}
	if s.cat == nil {
		log.Printf("[routing] no model catalog available — model ids in %s were NOT checked against the installed CLIs", where)
	} else {
		log.Printf("[routing] model ids in %s are checked against the installed CLIs by the routing service's own deferred catalog check, never as part of this load itself: the catalog is asked over the bus, and at boot the bus is not listening yet", where)
	}
}

// seedLocked writes the shipped default to disk the first time this machine is
// offered it, and never again.
//
// Both halves matter. A file the user is told to edit has to exist — so the
// embedded bytes go down verbatim, comments and all, header included. And a file
// deleted on purpose has to stay deleted — so the offer is recorded in a marker
// beside it, and a machine that already has a routing.yaml from before the
// marker existed simply records the fact without touching the file.
//
// Nothing here is fatal: a directory that cannot be created or a write that
// fails leaves the hub running on the compiled-in defaults, which is a complete,
// correct matrix.
func (s *Service) seedLocked() {
	if s.path == "" {
		return
	}
	marker := seedMarkerFor(s.path)
	if _, err := os.Stat(marker); err == nil {
		return // already offered, whatever the user did with it afterwards
	}
	if _, err := os.Stat(s.path); err == nil {
		// A pre-marker install. The file is the user's; record the offer so a
		// later delete is honoured, and do not write over it.
		s.writeMarker(marker)
		return
	} else if !os.IsNotExist(err) {
		log.Printf("[routing] cannot tell whether %s exists, not seeding: %v", s.path, err)
		return
	}
	if dir := filepath.Dir(s.path); dir != "" && dir != "." {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			log.Printf("[routing] cannot create %s, not seeding: %v", dir, err)
			return
		}
	}
	// 0600 for the same reason jobs.json is: this file decides how much
	// capability and how much autonomy a spawned agent gets.
	if err := os.WriteFile(s.path, defaultMatrixYAML, 0o600); err != nil {
		log.Printf("[routing] cannot write the default matrix to %s: %v", s.path, err)
		return
	}
	log.Printf("[routing] wrote the shipped default routing matrix to %s — edit it and it takes effect on the next tick", s.path)
	s.writeMarker(marker)
}

func (s *Service) writeMarker(marker string) {
	if err := os.WriteFile(marker, []byte("{\"seededVersion\":1}\n"), 0o600); err != nil {
		log.Printf("[routing] cannot record the seed marker %s (the default may be re-offered after a delete): %v", marker, err)
	}
}

// DefaultCatalogRetryEvery bounds how often a catalog check that could NOT get
// every provider's answer is tried again.
//
// The retry exists because "the daemon was down" and "there was no peer to ask"
// are both states somebody is expected to fix while the hub runs, exactly as
// cmd/hub's catalog cache says. The bound exists because asking costs a provider
// CLI boot: the catalog caches an unanswered provider for minutes, so a per-tick
// retry would mostly re-read a cache, and this keeps even the miss cheap.
const DefaultCatalogRetryEvery = 5 * time.Minute

// DefaultFirstCatalogCheckDelay is how long after Run starts the FIRST catalog
// check fires, ahead of DefaultTickEvery.
//
// Without this, the fallover walk (alternatives.go) is blind between boot and
// the first ordinary tick: it reads Matrix.Issues to decide whether a candidate
// is usable, and for up to DefaultTickEvery nothing has populated the catalog's
// half of Issues yet, so a catalog-invalid alternative reads as clean. A few
// seconds is enough for the desktop — which is what answers claude.listModels,
// and which only connects once /health answers — to have made that connection,
// without paying a full DefaultTickEvery of blindness on every boot.
const DefaultFirstCatalogCheckDelay = 5 * time.Second

// CatalogPending reports whether the live matrix still owes the catalog a check
// (either it has had none, or the last one ran while a provider it names could
// not answer).
func (s *Service) CatalogPending() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.catalogPending
}

// ValidateCatalog checks the live matrix's model ids against the injected
// catalog and folds the findings into Matrix.Issues, and reports whether a check
// actually ran.
//
// This is the half of the load that New deliberately does not do. It is separate
// because it does I/O and the boot path cannot afford it (see New), and it is
// idempotent because it may run more than once against the same document: the
// findings are rebuilt from the load-time ones every time rather than appended
// to what is already there.
//
// TWO THINGS ARE DELIBERATE ABOUT THE LOCKING. The probe runs with the mutex
// RELEASED, because a provider CLI can take twenty seconds to answer and
// routing.select reads Matrix() on the decision path: a check must never hold a
// decision open. And the result is published as a REPLACEMENT matrix rather than
// by appending to the live one, because a decision in flight is reading the
// Issues slice of the matrix it was handed.
func (s *Service) ValidateCatalog() bool {
	s.mu.Lock()
	m, cat, base := s.matrix, s.cat, s.baseIssues
	skip := !s.catalogPending || m == nil || cat == nil || s.catalogChecking ||
		(s.haveCatalogTry && s.retryEvery > 0 && time.Since(s.catalogTriedAt) < s.retryEvery)
	if skip {
		s.mu.Unlock()
		return false
	}
	// isRetry is true exactly when a check has already run against THIS
	// document and is being tried again — never on the first look at a
	// freshly installed or reloaded matrix (installLocked resets
	// haveCatalogTry). It is what forces a fresh probe rather than one: the
	// injected Catalog (cmd/hub's routingCatalog) caches a provider's answer
	// for catalogTTL (10m), longer than DefaultCatalogRetryEvery (5m), so a
	// retry that read the plain cache would reliably replay the same stale
	// miss it is retrying.
	isRetry := s.haveCatalogTry
	s.catalogChecking = true
	s.catalogTriedAt, s.haveCatalogTry = time.Now(), true
	s.mu.Unlock()

	probe := &answerCounting{cat: cat, force: isRetry}
	found := ValidateAgainstCatalog(m, probe)

	s.mu.Lock()
	defer s.mu.Unlock()
	s.catalogChecking = false
	if s.matrix != m {
		// The file was reloaded while we were asking. This verdict is about a
		// document that is no longer live, and the one that replaced it has its
		// own pending check.
		return false
	}
	next := *m
	next.Issues = make([]Issue, 0, len(base)+len(found))
	next.Issues = append(next.Issues, base...)
	next.Issues = append(next.Issues, found...)
	next.CatalogChecked = true
	s.matrix = &next
	// A provider that could not answer says nothing about whether the matrix is
	// right, so it leaves the check owed rather than settled.
	s.catalogPending = probe.unanswered
	if !sameIssues(s.catalogIssues, found) {
		for _, iss := range found {
			log.Printf("[routing] %s", iss)
		}
	}
	s.catalogIssues = found
	return true
}

// answerCounting wraps a Catalog to record whether any provider failed to give a
// usable answer. It is the only way to tell "checked" from "checked what it
// could": ValidateAgainstCatalog skips a provider it cannot get an answer for,
// by design, and skipping silently is what would make a pending check look done.
//
// It is also where a RETRY is told apart from a first look (see force):
// ValidateCatalog sets force on every call after the first for a given
// document, and this is the one seam ValidateAgainstCatalog calls through, so
// it is the one place that distinction can be turned into a forced probe.
type answerCounting struct {
	cat Catalog
	// force asks a RefreshingCatalog to re-probe rather than answer from its
	// own cache. Plain Catalog implementations (tests, mainly) have no cache
	// to force past, so this is a no-op for them.
	force      bool
	unanswered bool
}

func (a *answerCounting) Models(provider string) ([]CatalogModel, error) {
	models, err := a.probe(provider)
	if err != nil || len(models) == 0 {
		a.unanswered = true
	}
	return models, err
}

func (a *answerCounting) probe(provider string) ([]CatalogModel, error) {
	if a.force {
		if rc, ok := a.cat.(RefreshingCatalog); ok {
			return rc.Refresh(provider)
		}
	}
	return a.cat.Models(provider)
}

func sameIssues(a, b []Issue) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// Run polls the file until ctx ends, and runs the catalog check the boot path
// hands it. The hub's wiring owns the goroutine; this is here so the wiring is
// one line and the tick has one implementation.
//
// THE FIRST CATALOG CHECK FIRES ON ITS OWN SHORT TIMER, ahead of the ordinary
// DefaultTickEvery tick — see DefaultFirstCatalogCheckDelay. Waiting for the
// first 30-second tick left the fallover walk (alternatives.go's unusable)
// reading an unchecked Matrix.Issues for that whole window: a catalog-invalid
// alternative looked clean simply because nothing had asked the catalog about
// it yet. s.firstCheckDelay is a field, not a literal here, so a test can drive
// it without a real sleep.
func (s *Service) Run(ctx context.Context, every time.Duration) {
	if every <= 0 {
		every = DefaultTickEvery
	}
	firstDelay := s.firstCheckDelay
	if firstDelay <= 0 {
		firstDelay = DefaultFirstCatalogCheckDelay
	}
	first := time.NewTimer(firstDelay)
	defer first.Stop()
	t := time.NewTicker(every)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-first.C:
			// The first moment the catalog check can succeed: the listener is
			// up and the desktop, which answers claude.listModels, has had its
			// chance to connect — sooner than the ordinary tick, and still
			// nothing New itself waits on.
			s.ValidateCatalog()
		case <-t.C:
			s.ReloadIfChanged()
			// A no-op unless a hand edit re-armed the check (ReloadIfChanged
			// just installed a fresh document) or a provider could not be
			// reached and the check is still owed.
			s.ValidateCatalog()
		}
	}
}
