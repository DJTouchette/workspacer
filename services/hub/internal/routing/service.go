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
}

// New builds the service: seed the shipped default if this machine has never
// been offered it, then load.
//
// path "" disables the file entirely and runs on the compiled-in defaults, which
// is what tests and a read-only deployment want. cat may be nil, in which case
// model ids go unvalidated and that is said out loud once.
func New(path string, cat Catalog) *Service {
	s := &Service{path: path, cat: cat}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.seedLocked()
	// Boot is just the first reload: same read, same parse, same merge, same
	// validation, one code path.
	if !s.reloadIfChangedLocked() && s.matrix == nil {
		// No file, or a file that could not be read: run on the defaults.
		if m, err := Defaults(); err == nil {
			m.fallback, _ = Defaults()
			s.matrix = m
			log.Printf("[routing] running on the compiled-in default matrix (no usable %s)", s.describePath())
		} else {
			log.Printf("[routing] the compiled-in default matrix does not load: %v", err)
		}
	}
	return s
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
	m.Issues = append(m.Issues, ValidateAgainstCatalog(m, s.cat)...)
	s.matrix = m
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

// Run polls the file until ctx ends. The hub's wiring owns the goroutine; this
// is here so the wiring is one line and the tick has one implementation.
func (s *Service) Run(ctx context.Context, every time.Duration) {
	if every <= 0 {
		every = DefaultTickEvery
	}
	t := time.NewTicker(every)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			s.ReloadIfChanged()
		}
	}
}
