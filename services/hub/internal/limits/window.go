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

// Reading is one window, judged. The zero value is not meaningful and every
// accessor refuses it; construct it with ReadWindow.
//
// The invariant this type exists to make structural: UsedPercent, ResetsAt and
// TimeToReset are only ever readable when the verdict is WindowCurrent. There
// is no path through ReadWindow that fills any of them otherwise, so a caller
// cannot read a capacity off a dead window by forgetting to check the state -
// the worst it can do is read a zero, and the accessors below refuse even that.
//
// EVERY FIELD IS PRIVATE, and that is the fix for the hole the P0 review found.
// While `State` was exported, the verdict was FORGEABLE: `Reading{State:
// WindowCurrent, Now: time.Now()}` - written by hand, or produced by a JSON
// round trip, which preserves the exported half and drops the private half -
// answered ResetsAt() with ok=true and a ZERO reset time, so TimeToReset()
// returned a large NEGATIVE duration through a gate whose entire purpose is
// that a non-positive time-to-reset is unreachable. A verdict that can be
// asserted from outside is not a verdict. Now the only constructor is
// ReadWindow, an unmarshalled Reading is indistinguishable from the zero value,
// and the zero value is unusable.
//
// The belt-and-braces half: ResetsAt/TimeToReset additionally require the
// private reset time to have actually been populated (hasReset) AND the
// remaining duration to be strictly positive, so even a future edit that
// reintroduces a way to set the state cannot resurrect the negative-duration
// path.
type Reading struct {
	state WindowState
	// reason is empty when the verdict is WindowCurrent and is one of the
	// Reason* constants otherwise.
	reason string

	// usedPercent is nil when the window is current but the source could not
	// read a percentage for it. Currency and readability are separate axes:
	// the window may be the one running (so a time-to-reset is knowable) while
	// its utilization is not.
	usedPercent *float64
	resetsAt    time.Time
	// hasReset records that resetsAt was actually populated by ReadWindow, so
	// a zero time can never be mistaken for "the epoch, which is in the past
	// but present".
	hasReset bool
	// windowMinutes is the window's declared length, when the source reports
	// one. Anthropic now reports 300 (five_hour) and 10080 (seven_day) too, as
	// of the pace-aware routing change; codex has always reported 300 and
	// 10080. The monthly overage window still reports none, on either side.
	windowMinutes *int64

	// now is the instant the verdict was reached. Held so a decision log can
	// say which clock produced it - the whole failure this guard closes is a
	// verdict outliving the moment it was correct at.
	now  time.Time
	from Provenance
}

// State is the currency verdict. The zero Reading answers "" - not
// WindowCurrent, and not WindowUnreadable either: an unconstructed Reading is
// not a reading at all, and dressing it as a definite unknown would let one
// through a `switch` that handles the three real verdicts.
func (r Reading) State() WindowState { return r.state }

// Reason is empty on a current reading and one of the Reason* constants
// otherwise.
func (r Reading) Reason() string { return r.reason }

// Now is the instant this verdict was reached, which is the only clock any of
// its durations are relative to.
func (r Reading) Now() time.Time { return r.now }

// From is what the caller knew about where the reading came from.
func (r Reading) From() Provenance { return r.from }

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
	r := Reading{now: now, from: from}
	if w == nil {
		r.state, r.reason = WindowUnreadable, ReasonNoWindowReading
		return r
	}
	if w.ResetsAt == nil {
		// No reset time is no answer: a percentage alone cannot say WHICH
		// window it describes, which is exactly why the daemon leaves
		// is_current null here too. Note that the percentage is dropped even
		// when it reads `ok` - "67% and no reset" and "0% and no reset" are
		// both inventions.
		r.state, r.reason = WindowUnreadable, ReasonNoResetTime
		return r
	}
	resets := time.Unix(*w.ResetsAt, 0)
	switch {
	case resets.Equal(now):
		r.state, r.reason = WindowRolledOver, ReasonResetEqualsNow
		return r
	case resets.Before(now):
		r.state, r.reason = WindowRolledOver, ReasonResetHasPassed
		return r
	}
	r.state = WindowCurrent
	r.resetsAt, r.hasReset = resets, true
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
	if r.state != WindowCurrent || r.usedPercent == nil {
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
//
// THREE conditions, not one, and the last two are the P0 review's fix. The
// verdict alone used to be the whole gate, so anything that could assert the
// verdict - a hand-built literal, an unmarshalled document - got a zero
// resetsAt back with ok=true. Requiring hasReset means the field was actually
// populated by ReadWindow; requiring the reset to be strictly after the
// deciding instant means the answer is checked against the clock it will be
// subtracted from rather than trusted from a tag.
func (r Reading) ResetsAt() (time.Time, bool) {
	if r.state != WindowCurrent || !r.hasReset || !r.resetsAt.After(r.now) {
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
	d := resets.Sub(r.now)
	if d <= 0 {
		// Unreachable through ResetsAt above, and stated anyway: this is the
		// exact value the spec's `time_to_reset < 90 minutes` arm reads wrong,
		// so the function that produces it refuses rather than relying on one
		// caller upstream having got its comparison right.
		return 0, false
	}
	return d, true
}

// maxWindowMinutes bounds what WindowLength will trust. `int64(minutes) *
// time.Minute` overflows silently for an absurd value — time.Duration is an
// int64 count of nanoseconds, so anything above roughly 15250 years wraps —
// and long before that a "window" claiming to be, say, centuries long is not
// a length pacing was designed to divide by. 366 days is a full year with
// room for a leap one; nothing this package models is longer than a month.
const maxWindowMinutes = 366 * 24 * 60

// WindowLength is the window's declared duration, when the source reports
// one and that value is plausible. Anthropic reports none for the monthly
// overage window, so a caller must tolerate false here on a perfectly
// healthy claude reading; a windowMinutes above maxWindowMinutes is treated
// the same way — no length — rather than handed to a caller as a number that
// could overflow the arithmetic on the other end of it.
func (r Reading) WindowLength() (time.Duration, bool) {
	if r.state != WindowCurrent || r.windowMinutes == nil {
		return 0, false
	}
	m := *r.windowMinutes
	if m <= 0 || m > maxWindowMinutes {
		return 0, false
	}
	return time.Duration(m) * time.Minute, true
}

// Usable reports whether anything may be read off this reading at all.
func (r Reading) Usable() bool { return r.state == WindowCurrent }

// Explain is one clause for a routing decision's reason list (the spec's §31).
// Surfacing staleness is the cheapest detection this feature has: a provider
// that silently vanishes from a decision is indistinguishable from one that was
// considered and rejected.
func (r Reading) Explain() string {
	switch r.state {
	case WindowCurrent:
		d, ok := r.TimeToReset()
		if !ok {
			// Only reachable from a Reading nobody constructed. Say so rather
			// than printing "resets in 0s", which reads as a live window.
			return "unknown - this reading was never taken"
		}
		if used, ok := r.UsedPercent(); ok {
			return fmt.Sprintf("%.0f%% used, resets in %s", used, d.Round(time.Minute))
		}
		return fmt.Sprintf("utilization unreadable, resets in %s", d.Round(time.Minute))
	case WindowRolledOver:
		return "unknown - the last reading describes a window that has since rolled over"
	case WindowUnreadable:
		return "unknown - no reset time was reported, so no window can be identified"
	default:
		return "unknown - this reading was never taken"
	}
}
