// Package limits turns claudemon's GET /usage/report into facts a routing
// engine may act on. It decides nothing: what to do with a scarce or an
// expiring allowance is the routing layer's question, and this package's only
// job is to say honestly what is known and what is not.
//
// THIS PACKAGE IS HUB-NATIVE AND HAS NO TYPESCRIPT TWIN, deliberately. Every
// input it reads either belongs to the hub or arrives over the bus, and the hub
// is the one process that exists in every deployment (Electron spawns it,
// `workspacer serve` is it, a Fly node runs it). internal/quiescence is the
// structural precedent: a pure predicate here, the wiring in cmd/hub/. A limit
// reading reaches a client as an ANSWER, never as a recomputation.
//
// The one thing that IS twinned is the rule in this file, and only because a
// second reader of the same document already existed before routing did:
// apps/desktop/.../services/keepWarmLogic.ts fiveHourWindowFromReport. That
// function answers a NARROWER question - is a 5h window running, yes/no/unknown
// - and deliberately throws the percentage away, because keep-warm's
// windowActive() reads any percentage above zero as a live window and a stale
// 67% would suppress codex warming permanently. Routing needs the percentage,
// so this is a superset rather than a port. The shared half - the currency test
// itself - is pinned by contracts/usage-window-currency-cases.json, which both
// sides load.
package limits

import (
	"fmt"
	"time"
)

// ---------------------------------------------------------------------------
// THE WIRE
//
// The shape claudemon serves, mirrored from services/claudemon/src/session/
// usage_report.rs. Only the window half is typed here; the document is far
// wider (spend, tokens, per-model splits) and the fields this package does not
// consume are decoded in bucket.go where they belong to a bucket's provenance.
// ---------------------------------------------------------------------------

// MeasuredState is the tag of the daemon's three-state scalar.
type MeasuredState string

const (
	// MeasuredOk carries a real reading. The value may be 0, and that is an
	// answer.
	MeasuredOk MeasuredState = "ok"
	// MeasuredUnknown is not knowable right now. A retry may succeed.
	MeasuredUnknown MeasuredState = "unknown"
	// MeasuredUnavailable is not knowable at all, by anyone, from here. A retry
	// will not succeed.
	MeasuredUnavailable MeasuredState = "unavailable"
)

// Measured is one scalar in the report: a reading, or a reason there is not
// one. It is decoded as three states rather than flattened to a *float64 on
// purpose - the routing spec's `RemainingRatio *float64` has room for two of
// the three answers, and collapsing "the token expired, retry may fix it" into
// "GitHub will never tell you" produces `conserve copilot forever`.
type Measured struct {
	State  MeasuredState `json:"state"`
	Value  *float64      `json:"value,omitempty"`
	Reason string        `json:"reason,omitempty"`
}

// Number returns the reading and whether there is one. A Measured whose state
// is not ok has no number, however plausible its reason field looks.
func (m *Measured) Number() (float64, bool) {
	if m == nil || m.State != MeasuredOk || m.Value == nil {
		return 0, false
	}
	return *m.Value, true
}

// WireWindow is one rate-limit window as the report serves it.
type WireWindow struct {
	UsedPercent *Measured `json:"used_percent"`
	// ResetsAt is epoch seconds, or null when the source reported no reset
	// time.
	ResetsAt      *int64 `json:"resets_at"`
	WindowMinutes *int64 `json:"window_minutes"`
	// IsCurrent is a DISPLAY HINT AND IS NEVER A DECISION INPUT. The daemon
	// computes it against the same `now` that stamps generated_at, so it is
	// correct in the response and becomes a lie the moment a client caches or
	// polls the document - which is exactly what a routing engine does. It is
	// decoded so a UI can caption a reading and so the contract corpus can
	// carry the daemon's own answer beside the right one; ReadWindow ignores
	// it. See contracts/usage-window-currency-cases.json, case "the wire still
	// says is_current true and the reset has since passed".
	IsCurrent *bool `json:"is_current"`
}

// ---------------------------------------------------------------------------
// THE RULE
// ---------------------------------------------------------------------------

// WindowState is the currency verdict for one reading.
type WindowState string

const (
	// WindowCurrent means resets_at is present and strictly after the moment of
	// the decision. This is the ONLY state in which a percentage, a remaining
	// capacity or a time-to-reset may be read off the reading.
	WindowCurrent WindowState = "current"
	// WindowRolledOver means resets_at is present and at or before now: the
	// window this reading describes has closed. A DEFINITE answer - no window
	// from this reading is running - which is strictly more than "unknown".
	WindowRolledOver WindowState = "rolled-over"
	// WindowUnreadable means there is no usable resets_at at all, so nothing
	// can be said either way. Genuinely unknown; ask a different source.
	WindowUnreadable WindowState = "unreadable"
)

// The reasons a reading is not current. These strings are the vocabulary of
// contracts/usage-window-currency-cases.json and window_test.go holds the two
// sets equal in both directions, so a reason added here without a case - or a
// case naming a reason this file cannot produce - fails.
const (
	ReasonResetHasPassed  = "reset-time-has-passed"
	ReasonResetEqualsNow  = "reset-time-equals-now"
	ReasonNoResetTime     = "no-reset-time-reported"
	ReasonNoWindowReading = "no-window-reading-at-all"
)

// Provenance is what a caller already knows about where a reading came from,
// carried through onto the Reading so a routing decision can EXPLAIN itself
// ("codex 5h unknown - the last reading is on-disk and two days old") rather
// than silently omitting a provider. bucket.go supplies it from the account
// row; nothing in this file derives it.
type Provenance struct {
	// Source is the wire's own word: oauth_poll, disk or transcript. Kept
	// verbatim rather than mapped onto the routing spec's
	// provider_reported/cli_status/usage_inferred names, because the wire word
	// is the one a reader can check against the document.
	Source string
	// ObservedAt is when the underlying reading was taken, if known. It is NOT
	// the currency test - a fresh observation of a dead window is still a dead
	// window, and a two-day-old observation of a weekly window may be perfectly
	// current. It is here so a decision can say how old its evidence is.
	ObservedAt *time.Time
}

// Reading is one window, judged. The zero value is not meaningful; construct it
// with ReadWindow.
//
// The invariant this type exists to make structural: UsedPercent, ResetsAt and
// TimeToReset are only ever populated when State is WindowCurrent. There is no
// path through ReadWindow that fills any of them otherwise, so a caller cannot
// read a capacity off a dead window by forgetting to check the state - the
// worst it can do is read a zero, and the accessors below refuse even that.
type Reading struct {
	State WindowState
	// Reason is empty when State is WindowCurrent and is one of the Reason*
	// constants otherwise.
	Reason string

	// usedPercent is nil when the window is current but the source could not
	// read a percentage for it. Currency and readability are separate axes:
	// the window may be the one running (so a time-to-reset is knowable) while
	// its utilization is not.
	usedPercent *float64
	resetsAt    time.Time
	// windowMinutes is the window's declared length, when the source reports
	// one. Anthropic reports none; codex reports 300 and 10080.
	windowMinutes *int64

	// Now is the instant the verdict was reached. Held so a decision log can
	// say which clock produced it - the whole failure this guard closes is a
	// verdict outliving the moment it was correct at.
	Now  time.Time
	From Provenance
}

// ReadWindow is THE currency test, and the only door a percentage may come
// through.
//
//	A window reading is usable ONLY IF resets_at is present AND resets_at is
//	strictly after `now` AT THE MOMENT OF THE DECISION. A reading that fails
//	that test yields UNKNOWN for that window - never a percentage, never a
//	remaining capacity, never a time-to-reset.
//
// The comparison is strictly greater-than, matching what
// services/claudemon/src/providers/codex_usage.rs already pins (is_current is
// false AT the reset), so the two sides cannot disagree by one second.
//
// Signedness is handled explicitly rather than left to the caller. The routing
// spec's initial spend-down rule reads `time_to_reset < 90 minutes`, which is
// trivially true on a negative number: live on 2026-08-30 codex reported its 5h
// window used 67.0% against a resets_at 170569 seconds in the past, and a
// literal implementation of that arm spends down premium allowance against a
// window that closed two days ago. TimeToReset below can never return a
// negative duration because it can only be reached in WindowCurrent.
func ReadWindow(w *WireWindow, from Provenance, now time.Time) Reading {
	r := Reading{Now: now, From: from}
	if w == nil {
		r.State, r.Reason = WindowUnreadable, ReasonNoWindowReading
		return r
	}
	if w.ResetsAt == nil {
		// No reset time is no answer: a percentage alone cannot say WHICH
		// window it describes, which is exactly why the daemon leaves
		// is_current null here too. Note that the percentage is dropped even
		// when it reads `ok` - "67% and no reset" and "0% and no reset" are
		// both inventions.
		r.State, r.Reason = WindowUnreadable, ReasonNoResetTime
		return r
	}
	resets := time.Unix(*w.ResetsAt, 0)
	switch {
	case resets.Equal(now):
		r.State, r.Reason = WindowRolledOver, ReasonResetEqualsNow
		return r
	case resets.Before(now):
		r.State, r.Reason = WindowRolledOver, ReasonResetHasPassed
		return r
	}
	r.State = WindowCurrent
	r.resetsAt = resets
	r.windowMinutes = w.WindowMinutes
	if v, ok := w.UsedPercent.Number(); ok {
		r.usedPercent = &v
	}
	return r
}

// UsedPercent is the utilization a router may act on, and whether there is one.
// False on any reading that is not current, and false on a current reading
// whose source could not read a percentage.
func (r Reading) UsedPercent() (float64, bool) {
	if r.State != WindowCurrent || r.usedPercent == nil {
		return 0, false
	}
	return *r.usedPercent, true
}

// RemainingPercent is the complement of UsedPercent, on the same terms. It is
// derived here rather than by each caller so that "1 - used/100" cannot be
// computed off a reading that has no used.
func (r Reading) RemainingPercent() (float64, bool) {
	used, ok := r.UsedPercent()
	if !ok {
		return 0, false
	}
	return 100 - used, true
}

// ResetsAt is when the window lapses, and whether that is known. Only a current
// reading has one: a rolled-over reading's reset is in the past and describes a
// window that no longer exists.
func (r Reading) ResetsAt() (time.Time, bool) {
	if r.State != WindowCurrent {
		return time.Time{}, false
	}
	return r.resetsAt, true
}

// TimeToReset is how long the running window has left. It can never be
// negative or zero: WindowCurrent is only reachable when resets_at is strictly
// after the deciding instant, which is the guard the spec's
// `time_to_reset < 90 minutes` arm lacks.
func (r Reading) TimeToReset() (time.Duration, bool) {
	resets, ok := r.ResetsAt()
	if !ok {
		return 0, false
	}
	return resets.Sub(r.Now), true
}

// WindowLength is the window's declared duration, when the source reports one.
// Anthropic reports none, so a caller must tolerate false here on a perfectly
// healthy claude reading.
func (r Reading) WindowLength() (time.Duration, bool) {
	if r.State != WindowCurrent || r.windowMinutes == nil {
		return 0, false
	}
	return time.Duration(*r.windowMinutes) * time.Minute, true
}

// Usable reports whether anything may be read off this reading at all.
func (r Reading) Usable() bool { return r.State == WindowCurrent }

// Explain is one clause for a routing decision's reason list (the spec's §31).
// Surfacing staleness is the cheapest detection this feature has: a provider
// that silently vanishes from a decision is indistinguishable from one that was
// considered and rejected.
func (r Reading) Explain() string {
	switch r.State {
	case WindowCurrent:
		d, _ := r.TimeToReset()
		if used, ok := r.UsedPercent(); ok {
			return fmt.Sprintf("%.0f%% used, resets in %s", used, d.Round(time.Minute))
		}
		return fmt.Sprintf("utilization unreadable, resets in %s", d.Round(time.Minute))
	case WindowRolledOver:
		return "unknown - the last reading describes a window that has since rolled over"
	default:
		return "unknown - no reset time was reported, so no window can be identified"
	}
}
