package limits

// The normalized limit bucket: claudemon's GET /usage/report, decoded into the
// unit the routing layer reasons about.
//
// THE KEY IS (provider, account, window), not (provider, window). The routing
// spec's ProviderLimitState has no account dimension and the real data does:
// live on 2026-08-30 this machine carried three claude account rows at once —
// `default` reading 18% on a running 5h window, `work` with an expired OAuth
// token and no readable window, and `unattributed` folded from transcripts that
// name no account. A provider-level health that folds those three together is a
// lie in either direction, and the account a spawn will actually bill to is
// already known at spawn time (profileId / CLAUDE_CONFIG_DIR), so the bucket is
// keyed the way the decision is made.
//
// TWO DELIBERATE DEPARTURES FROM THE SPEC'S LimitBucket, both recorded in
// .workspacer/reports/2026-08-30-routing-placement-plan.md §0.2:
//
//   - `RemainingRatio *float64` is not used. A pointer has room for two of the
//     three answers the daemon already distinguishes, and collapsing "the token
//     expired, a retry may fix it" into "GitHub will never tell you" produces
//     `conserve copilot forever`. The three-state Measured is carried through
//     intact and the guarded accessors live on Reading.
//   - `Confidence float64` is not used. `fresh`, `observed_at`,
//     `failure{kind,detail,at}` and the wire's `is_current` are four orthogonal
//     facts and a scalar cannot express the one that actually bit: "the reading
//     is real but describes a window that has closed".
//
// `ModelScope` IS modelled and is always empty, because nothing reports a
// per-model allowance today — the report's `models[]` carries per-model SPEND,
// not a per-model LIMIT. The spec's §33 "Fable allowance: YELLOW" has no data
// source; the field exists so the shape does not have to change when a manual
// bucket can express it.

import (
	"encoding/json"
	"fmt"
	"time"
)

// The three windows every provider is asked about, so a caller can lay out one
// row without knowing which provider fills which. The strings are the wire's
// own keys.
const (
	WindowFiveHour = "five_hour"
	WindowSevenDay = "seven_day"
	WindowMonthly  = "monthly"
)

// WindowOrder is the order buckets are produced in, shortest window first.
var WindowOrder = []string{WindowFiveHour, WindowSevenDay, WindowMonthly}

// ---------------------------------------------------------------------------
// THE WIRE — mirrored from services/claudemon/src/session/usage_report.rs.
// ---------------------------------------------------------------------------

// WireFailure is the daemon's classified account-level failure. Kept as a kind
// PLUS a detail because "needs_reauth" and "oauth token expired" together are
// strictly more than either alone.
type WireFailure struct {
	Kind   string `json:"kind"`
	Detail string `json:"detail"`
	At     int64  `json:"at"`
}

// WireWindows is one account's three windows.
type WireWindows struct {
	FiveHour *WireWindow `json:"five_hour"`
	SevenDay *WireWindow `json:"seven_day"`
	Monthly  *WireWindow `json:"monthly"`
}

// Window returns the named window, or nil. A nil is a real answer — see
// ReasonNoWindowReading.
func (w WireWindows) Window(name string) *WireWindow {
	switch name {
	case WindowFiveHour:
		return w.FiveHour
	case WindowSevenDay:
		return w.SevenDay
	case WindowMonthly:
		return w.Monthly
	}
	return nil
}

// WireAccount is one account row.
type WireAccount struct {
	// Account is the account key. `""` is the default login — a real answer.
	// `null` is UNKNOWN: the daemon cannot say which account this is, and a
	// consumer must render it as unknown rather than adding it to the default.
	Account   *string `json:"account"`
	Label     string  `json:"label"`
	IsDefault bool    `json:"is_default"`
	// Source is oauth_poll, disk or transcript.
	Source     string       `json:"source"`
	ObservedAt *int64       `json:"observed_at"`
	Fresh      *bool        `json:"fresh"`
	Failure    *WireFailure `json:"failure"`
	Windows    WireWindows  `json:"windows"`
}

// WireProvider is one provider's accounts, plus why there are none when there
// are none — an empty list alone is indistinguishable from a provider nobody
// uses.
type WireProvider struct {
	Provider string        `json:"provider"`
	Accounts []WireAccount `json:"accounts"`
	Note     *string       `json:"note"`
}

// WireReport is the whole document.
type WireReport struct {
	GeneratedAt int64          `json:"generated_at"`
	Providers   []WireProvider `json:"providers"`
}

// ---------------------------------------------------------------------------
// THE BUCKET
// ---------------------------------------------------------------------------

// Bucket is one (provider, account, window) allowance, judged.
type Bucket struct {
	Provider string
	// Account is the account key; `""` is the default login and is a real
	// answer. AccountKnown is false when the daemon could not say which account
	// the row describes, which is NOT the same as the default account.
	Account      string
	AccountKnown bool
	// AccountLabel is the short human name: "default", a profile directory's
	// basename, or "unattributed".
	AccountLabel string
	IsDefault    bool
	Window       string

	// ModelScope is which models this allowance covers. Always empty today; see
	// the package note.
	ModelScope []string

	// Reading is the CURRENCY-GUARDED reading and the only thing a routing
	// decision may act on. It was judged against the `now` passed to
	// Snapshot.Buckets, not against the instant the document was fetched.
	Reading Reading

	// RawUsedPercent is the wire scalar exactly as served, ungated.
	//
	// DISPLAY AND EXPLANATION ONLY. It is deliberately reachable so a decision
	// can say "codex 5h unknown — the last reading was 67% and its window has
	// since rolled over" instead of silently dropping the provider, which is
	// the cheapest detection this feature has. It is NEVER a routing input:
	// Reading is, and Reading refuses to hand out a percentage off a window
	// that has closed.
	RawUsedPercent Measured

	// Provenance of the account row this bucket came from.
	Source     string
	ObservedAt *time.Time
	// Fresh is the daemon's own freshness verdict, when the source has one. A
	// disk fold has none, which is why this is a pointer and not a bool.
	Fresh   *bool
	Failure *WireFailure
}

// ID is the spec's LimitBucket.ID, derived rather than carried on the wire.
// An unknown account is spelled `?` so it can never collide with the default
// login, which is spelled `""` and is a real answer.
func (b Bucket) ID() string {
	account := "?"
	if b.AccountKnown {
		account = b.Account
	}
	return fmt.Sprintf("%s/%s/%s", b.Provider, account, b.Window)
}

// Metered reports whether this bucket describes a subscription allowance that
// can be conserved at all. A bucket whose reading is unreadable AND whose raw
// scalar is structurally `unavailable` is a provider that will never publish a
// window — copilot, and every BYO-key provider absent from the document — and
// conserving one of those conserves nothing.
//
// This is a FACT about the reading, not the policy: what to do about an
// unmetered provider (the routing spec's per-provider `when_unknown`) is the
// routing layer's decision and is not made here.
func (b Bucket) Metered() bool {
	if b.Reading.Usable() {
		return true
	}
	return b.RawUsedPercent.State != MeasuredUnavailable
}

// ---------------------------------------------------------------------------
// THE SNAPSHOT
// ---------------------------------------------------------------------------

// Snapshot is one fetched /usage/report, held UNJUDGED.
//
// This is the shape that closes the cached-document hole. The currency test has
// to run at the moment of the DECISION, not at the moment of the fetch: a
// document fetched at T and judged at T is correct, and the same document
// consulted at T+40m can contain a window that lapsed in between. So a Snapshot
// retains the wire and every verdict is produced by Buckets(now) with the
// caller's own clock. There is deliberately no field on Snapshot holding
// pre-judged buckets, because a cached verdict is the entire defect.
type Snapshot struct {
	// GeneratedAt is when the daemon built the document.
	GeneratedAt time.Time
	// FetchedAt is when this process read it. Held so a caller can say how old
	// its evidence is; it is NOT the currency test.
	FetchedAt time.Time

	report WireReport
}

// DecodeReport parses a /usage/report body. It judges nothing.
func DecodeReport(raw []byte, fetchedAt time.Time) (Snapshot, error) {
	var wire WireReport
	if err := json.Unmarshal(raw, &wire); err != nil {
		return Snapshot{}, fmt.Errorf("decode usage report: %w", err)
	}
	s := Snapshot{FetchedAt: fetchedAt, report: wire}
	if wire.GeneratedAt > 0 {
		s.GeneratedAt = time.Unix(wire.GeneratedAt, 0)
	}
	return s, nil
}

// Empty reports whether the document carried no provider rows at all — a hub
// that has never fetched, or a daemon that answered `{}`. Distinct from a
// document whose providers all report nothing readable.
func (s Snapshot) Empty() bool { return len(s.report.Providers) == 0 }

// Providers lists the provider names the document carries, in document order.
// A provider ABSENT from this list is not "unknown at 0%" — it is not modelled
// at all (opencode and pi never appear), and a caller must tell the two apart.
func (s Snapshot) Providers() []string {
	out := make([]string, 0, len(s.report.Providers))
	for _, p := range s.report.Providers {
		out = append(out, p.Provider)
	}
	return out
}

// Note returns the daemon's reason a provider carries no accounts, if it gave
// one.
func (s Snapshot) Note(provider string) (string, bool) {
	for _, p := range s.report.Providers {
		if p.Provider == provider && p.Note != nil {
			return *p.Note, true
		}
	}
	return "", false
}

// Buckets judges every window in the document against `now` — the caller's
// clock at the moment of the decision. Call it per decision; do not cache what
// it returns.
func (s Snapshot) Buckets(now time.Time) []Bucket {
	var out []Bucket
	for _, p := range s.report.Providers {
		for _, a := range p.Accounts {
			for _, w := range WindowOrder {
				out = append(out, bucketFrom(p.Provider, a, w, now))
			}
		}
	}
	return out
}

// Bucket judges one window for a NAMED account — `""` being the default login.
// It deliberately cannot reach the row whose account the daemon could not
// determine: that row describes usage nobody can attribute, and a spawn always
// names the account it will bill to (profileId / CLAUDE_CONFIG_DIR), so a
// lookup that silently fell back to it would bill a decision to the wrong
// allowance. Iterate Buckets to see the unattributed row.
func (s Snapshot) Bucket(now time.Time, provider, account, window string) (Bucket, bool) {
	for _, p := range s.report.Providers {
		if p.Provider != provider {
			continue
		}
		for _, a := range p.Accounts {
			if a.Account == nil || *a.Account != account {
				continue
			}
			return bucketFrom(p.Provider, a, window, now), true
		}
	}
	return Bucket{}, false
}

func bucketFrom(provider string, a WireAccount, window string, now time.Time) Bucket {
	b := Bucket{
		Provider:     provider,
		AccountLabel: a.Label,
		IsDefault:    a.IsDefault,
		Window:       window,
		ModelScope:   nil,
		Source:       a.Source,
		Fresh:        a.Fresh,
		Failure:      a.Failure,
	}
	if a.Account != nil {
		b.Account, b.AccountKnown = *a.Account, true
	}
	if a.ObservedAt != nil {
		t := time.Unix(*a.ObservedAt, 0)
		b.ObservedAt = &t
	}
	w := a.Windows.Window(window)
	if w != nil && w.UsedPercent != nil {
		b.RawUsedPercent = *w.UsedPercent
	} else {
		// No scalar at all is not `ok 0`. Spelling it `unknown` with a reason
		// keeps the document's own three-state discipline through a hole in the
		// document.
		b.RawUsedPercent = Measured{
			State:  MeasuredUnknown,
			Reason: "the report carried no used_percent for this window",
		}
	}
	b.Reading = ReadWindow(w, Provenance{Source: a.Source, ObservedAt: b.ObservedAt}, now)
	return b
}
