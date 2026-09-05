// Package usageprefs holds the ONE user preference Overview's weekly pacing
// needs and routing.yaml deliberately cannot carry: whether this fleet's week
// is five weekdays or seven calendar days.
//
// WHY A SEPARATE FILE AT ALL, when `thresholds.pacing.seven_day.curve` already
// exists. routing.yaml is hub-owned state with NO write RPC over the bus, and
// that absence is load-bearing: together with fs.write refusing the hub's state
// directory it is the entire argument for routing.yaml's `ceilings:` block
// being a ceiling rather than a suggestion (cmd/hub/main.go says so where
// usage.report and routing.select are registered). A Settings toggle that
// edited routing.yaml would reverse that, so this file is the same shape the
// hub already uses for a preference a client may set: a small 0600 sibling of
// jobs.json with its own trusted-only RPC, exactly as internal/jobs is.
//
// WHAT IT CHANGES, and the precedence — this is the whole contract:
//
//	(absent / unreadable / unrecognised)  routing.yaml answers, unmodified. The
//	                                      installed matrix is the authority and
//	                                      the behaviour is byte-for-byte what it
//	                                      was before this file existed.
//	seven_day                             the CALENDAR curve, explicitly. A user
//	                                      who picked "every day" gets the
//	                                      calendar shape even if routing.yaml
//	                                      says `curve: workdays`.
//	five_day                              the FIVE_DAY curve: weekend weight
//	                                      zero, spend_tail, no reserve. Expected
//	                                      progress advances Monday–Friday and is
//	                                      flat across the weekend.
//
// The five-hour and monthly windows are untouched by any of it, and not by
// politeness: limits.expectedShare only consults a curve for WindowSevenDay,
// and the monthly overage window reports no length so PaceFor answers unknown
// before a curve is ever selected.
//
// WHAT IT DOES NOT DO. It writes nothing but its own file, it is never read as
// config, code, argv or policy by anything else, and routing.select does not
// consult it at all — routing decisions keep reading Matrix.PaceConfig()
// straight off routing.yaml, so an operator's routing posture cannot be moved
// from a Settings checkbox.
package usageprefs

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// The two-valued vocabulary, and the third state that is NOT a value.
const (
	// ScheduleUnset is "nobody has chosen", and it is distinct from
	// ScheduleSevenDay on purpose: unset preserves whatever routing.yaml says
	// (including `curve: workdays`, which an operator may have hand-set), while
	// seven_day is a user asserting the calendar shape over it.
	ScheduleUnset = ""
	// ScheduleFiveDay is "Work week (Monday–Friday)".
	ScheduleFiveDay = "five_day"
	// ScheduleSevenDay is "Every day (7 days)".
	ScheduleSevenDay = "seven_day"
)

// Validate normalizes a caller's value or refuses it. The enum is closed: a
// value nobody recognises is an ERROR at the setter rather than a silent
// fallback, because a Settings control that reports success and stored nothing
// is the failure mode this whole plumbing exists to avoid.
func Validate(v string) (string, error) {
	switch strings.ToLower(strings.TrimSpace(v)) {
	case ScheduleFiveDay:
		return ScheduleFiveDay, nil
	case ScheduleSevenDay:
		return ScheduleSevenDay, nil
	}
	return "", fmt.Errorf("usage pacing schedule %q is not %s or %s", v, ScheduleFiveDay, ScheduleSevenDay)
}

// file is the on-disk shape. One field, so a hand-edit is obvious.
type file struct {
	Schedule string `json:"schedule"`
}

// Store is the hub's copy of the preference: read once at construction, held in
// memory behind a mutex, written through on every change.
//
// A nil *Store is a usable Store meaning "unset" — that is what lets the hub be
// started with the preference file disabled (empty path) without every reader
// growing a branch.
type Store struct {
	mu       sync.RWMutex
	path     string
	schedule string
}

// Open reads the preference file if it is there.
//
// EVERY failure answers ScheduleUnset rather than an error, and that is the
// same policy routing/service.go states for a matrix it cannot read: a file
// that is absent, unreadable, malformed, or carries a word this build does not
// know leaves the behaviour exactly as it was. The err return is diagnostic —
// the caller logs it — and never a reason to refuse to boot, because a hub that
// will not start over an unparseable two-line preference is worse than one that
// paces on the calendar curve.
func Open(path string) (*Store, error) {
	s := &Store{path: path}
	if strings.TrimSpace(path) == "" {
		return s, nil
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return s, nil
		}
		return s, fmt.Errorf("usage pacing preference %s: %w", path, err)
	}
	var f file
	if err := json.Unmarshal(raw, &f); err != nil {
		return s, fmt.Errorf("usage pacing preference %s is not readable JSON (%w) — no schedule is applied and routing.yaml answers", path, err)
	}
	v, err := Validate(f.Schedule)
	if err != nil {
		return s, fmt.Errorf("usage pacing preference %s: %w — no schedule is applied and routing.yaml answers", path, err)
	}
	s.schedule = v
	return s, nil
}

// Schedule is the stored value, or ScheduleUnset.
func (s *Store) Schedule() string {
	if s == nil {
		return ScheduleUnset
	}
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.schedule
}

// Path is where the preference persists, for the reader RPC to report. Empty
// means this hub was started with the preference disabled.
func (s *Store) Path() string {
	if s == nil {
		return ""
	}
	return s.path
}

// Set validates, persists, and only THEN updates the in-memory value.
//
// The order is the point: a write that fails must leave the running hub and the
// next restart agreeing with each other, so a caller that is told the save
// failed is looking at the same schedule it had before. Written to a temp file
// in the same directory and renamed, 0600, exactly as internal/jobs persists —
// a half-written preference is a preference that reads as unset on the next
// boot.
func (s *Store) Set(v string) (string, error) {
	if s == nil {
		return "", fmt.Errorf("usage pacing schedule cannot be stored: this hub has no preference file")
	}
	schedule, err := Validate(v)
	if err != nil {
		return "", err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.path == "" {
		return "", fmt.Errorf("usage pacing schedule cannot be stored: this hub has no preference file")
	}
	raw, err := json.MarshalIndent(file{Schedule: schedule}, "", "  ")
	if err != nil {
		return "", fmt.Errorf("usage pacing schedule: %w", err)
	}
	if err := os.MkdirAll(filepath.Dir(s.path), 0o700); err != nil {
		return "", fmt.Errorf("usage pacing schedule: %w", err)
	}
	tmp := s.path + ".tmp"
	if err := os.WriteFile(tmp, append(raw, '\n'), 0o600); err != nil {
		return "", fmt.Errorf("usage pacing schedule: %w", err)
	}
	if err := os.Rename(tmp, s.path); err != nil {
		_ = os.Remove(tmp)
		return "", fmt.Errorf("usage pacing schedule: %w", err)
	}
	s.schedule = schedule
	return schedule, nil
}

// Apply is the ONE formula turning the stored preference into pacing inputs.
//
// It is a method on the store and a pure function of (schedule, cfg), so the
// handler, the tests and any future reader cannot each grow their own version
// of the precedence table in the package doc.
func (s *Store) Apply(cfg limits.PaceConfig) limits.PaceConfig {
	return ApplySchedule(s.Schedule(), cfg)
}

// ApplySchedule is Apply without a store, for callers that already hold the
// value (and for tests that want to state both arms in one table).
func ApplySchedule(schedule string, cfg limits.PaceConfig) limits.PaceConfig {
	switch schedule {
	case ScheduleFiveDay:
		cfg.Curve = limits.CurveFiveDay
		// The weekend knobs are the matrix's answer to a question this schedule
		// has already answered: at weight zero there is no weekend budget to
		// weight and nothing left for a reserve to hold back. Neutralized HERE,
		// once, so `weekend: reserve` in an operator's routing.yaml cannot
		// silently scale a curve the user asked to be flat.
		cfg.WeekendWeight = 0
		cfg.WeekendPolicy = limits.WeekendSpendTail
		cfg.WeekendReservePct = 0
	case ScheduleSevenDay:
		// "Every day" is the calendar shape by definition, so it overrides a
		// hand-set `curve: workdays` rather than inheriting it. Every other
		// field — bands, bootstrap, timezone, enabled — stays the matrix's.
		cfg.Curve = limits.CurveCalendar
	}
	// ScheduleUnset falls through untouched: routing.yaml is the authority
	// until somebody chooses.
	return cfg
}
