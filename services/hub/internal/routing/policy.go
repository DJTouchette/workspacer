package routing

// THE POLICY LAYER: capacity in, a routing decision out.
//
// This file is pure. It reads the matrix (which the service holds) and a
// limits.Snapshot (which the hub's usage edge holds), and it returns a
// Decision. It does no I/O, opens no socket, and reads no clock of its own —
// `now` is an argument, because the entire founding defect of this feature is a
// verdict outliving the moment it was correct at.
//
// THE TWO RULES THAT SURVIVE FROM P0, restated where they are enforced:
//
//  1. THE CURRENCY PRECONDITION COMES BEFORE ANY THRESHOLD COMPARISON. The
//     spec's §14 reads `time_to_reset < 90 minutes` literally, and a literal
//     implementation is WRONG on a negative number: on 2026-08-30 codex served
//     67% used against a resets_at 170569 seconds in the PAST, and the naive
//     arm spends down premium allowance against a window that closed two days
//     ago. There is no `< 90 minutes` comparison in this file that a stale
//     reading can reach, and the way that is achieved is by TYPE rather than by
//     a hand-written re-check: the only time-to-reset available here is
//     limits.BucketReport.ResetsInSeconds, which limits.BucketHealth populates
//     solely from Reading.TimeToReset — an accessor that cannot return a
//     non-positive duration. Do not add a second path to a duration.
//
//  2. AN UNKNOWN BUCKET IS UNKNOWN, and it never quietly becomes healthy.
//     Three of five providers have no readable quota permanently: copilot
//     answers 403 to the only endpoint that would tell it, and opencode and pi
//     never appear in the usage document at all. limits.Worst ranks UNKNOWN
//     above GREEN so a readable weekly window cannot vouch for an unreadable
//     five-hour one, and what to DO about an unknown provider is read from the
//     matrix (`providers[].when_unknown`) rather than assumed — which is why
//     the decision reports the OBSERVED health and the ASSUMED health as two
//     separate fields. Collapsing them is the bug.
//
// EVERY THRESHOLD IN THIS FILE IS AN ARGUMENT. 70, 90 and the 90-minute
// spend-down window live in routing.yaml under `thresholds:`; the phase weights
// live under `forecast_weights:`; the manual override lives under `modes:`; and
// the mode-driven capability moves live under `mode_shifts:`. There is no
// tunable number in this source file, on purpose.

import (
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/limits"
)

// Mode is the routing spec's §10: given the capacity and the time, how
// aggressively should it be spent?
type Mode string

const (
	// ModeNormal is the ordinary policy: the matrix's own table.
	ModeNormal Mode = "normal"
	// ModeConserve means the allowance is scarce or is being consumed faster
	// than it can be replaced: be selective about premium capability.
	ModeConserve Mode = "conserve"
	// ModeSpendDown means a reset is near with capacity left and little demand
	// coming, so the expiring remainder should buy confidence — a stronger
	// scout, a second independent reviewer — rather than expire unused.
	ModeSpendDown Mode = "spend_down"
	// ModeAuto is not a mode. It is what the matrix's `modes:` block says when
	// it is deferring to the thresholds, and it never appears on a Decision.
	ModeAuto Mode = "auto"
)

// ParseMode reads a mode out of the matrix's `modes:` block.
func ParseMode(s string) (Mode, bool) {
	switch Mode(strings.ToLower(strings.TrimSpace(s))) {
	case ModeNormal:
		return ModeNormal, true
	case ModeConserve:
		return ModeConserve, true
	case ModeSpendDown:
		return ModeSpendDown, true
	case ModeAuto:
		return ModeAuto, true
	}
	return "", false
}

// ---------------------------------------------------------------------------
// CAPACITY
// ---------------------------------------------------------------------------

// Capacity is one provider's limit picture at the deciding instant, folded and
// explained.
//
// Health and EffectiveHealth are DELIBERATELY SEPARATE FIELDS. Health is what
// the document actually supports; EffectiveHealth is Health unless Health is
// UNKNOWN, in which case it is whatever the matrix's `when_unknown` says to
// assume. Reporting only the second one is how "we could not read codex" turns
// into "codex is fine" three layers downstream, and reporting only the first
// leaves the policy with nothing to act on. A reader gets both, always.
type Capacity struct {
	Provider string `json:"provider"`
	// Account is the account row the decision drew on; "" is the default login
	// and is a real answer. AccountKnown is false when no attributable row for
	// this provider exists in the document at all.
	Account      string `json:"account"`
	AccountKnown bool   `json:"accountKnown"`

	// Health is the OBSERVED fold across this account's meaningful buckets.
	Health limits.Health `json:"health"`
	// AssumedHealth is the matrix's `providers[].when_unknown`, present only
	// when Health is unknown.
	AssumedHealth limits.Health `json:"assumedHealth,omitempty"`
	// EffectiveHealth is what the mode rules below actually compared against.
	EffectiveHealth limits.Health `json:"effectiveHealth"`

	// Metered is whether this provider publishes any allowance at all.
	Metered bool                  `json:"metered"`
	Buckets []limits.BucketReport `json:"buckets,omitempty"`

	// Because is the sentence behind Health.
	Because string `json:"because"`
	// Note is the daemon's own explanation for a provider carrying no accounts,
	// when it gave one.
	Note string `json:"note,omitempty"`
	// InReport is false when the provider does not appear in the usage document
	// at all. Distinct from a provider that appears and reports nothing: the
	// first is "not modelled", the second is "modelled and dark", and a caller
	// that cannot tell them apart will wait for a reading that is never coming.
	InReport bool `json:"inReport"`
	// ObservedAt is the wire's own timestamp for the reading behind this
	// account, when it had one — so a decision can say how old its evidence is.
	ObservedAt *int64 `json:"observedAt,omitempty"`
}

// ReadCapacity judges the snapshot for one provider and account, at `now`.
//
// It calls Snapshot.Buckets(now) rather than anything cached, because a cached
// verdict is the same defect one level up: correct at fetch time, wrong at
// decision time. Nothing in this package holds a judged bucket between calls.
func ReadCapacity(m *Matrix, snap limits.Snapshot, provider, account string, now time.Time) Capacity {
	provider = normalizeProvider(provider)
	cap := Capacity{Provider: provider}

	buckets := snap.Buckets(now)
	for _, p := range snap.Providers() {
		if p == provider {
			cap.InReport = true
			break
		}
	}
	if note, ok := snap.Note(provider); ok {
		cap.Note = note
	}

	resolved, known := limits.ResolveAccount(buckets, provider, account)
	cap.Account, cap.AccountKnown = resolved, known

	bands := limits.Bands{
		YellowAtUsedPct: m.Thresholds.Health.YellowAtUsedPct,
		RedAtUsedPct:    m.Thresholds.Health.RedAtUsedPct,
	}

	if known {
		rep := limits.ForProvider(buckets, provider, resolved, bands)
		cap.Health, cap.Because, cap.Metered, cap.Buckets = rep.Health, rep.Because, rep.Metered, rep.Buckets
		for _, b := range buckets {
			if b.Provider == provider && b.AccountKnown && b.Account == resolved && b.ObservedAt != nil {
				at := b.ObservedAt.Unix()
				cap.ObservedAt = &at
				break
			}
		}
	} else {
		cap.Health = limits.HealthUnknown
		switch {
		case !cap.InReport:
			cap.Because = fmt.Sprintf(
				"%s does not appear in claudemon's usage report at all, so its capacity is unknown — it is not modelled, which is a different answer from 'modelled and reporting nothing'",
				provider)
		case cap.Note != "":
			cap.Because = fmt.Sprintf("%s carries no attributable account row (%s), so its capacity is unknown", provider, cap.Note)
		default:
			cap.Because = fmt.Sprintf("%s carries no attributable account row, so its capacity is unknown", provider)
		}
	}

	cap.EffectiveHealth = cap.Health
	if cap.Health == limits.HealthUnknown {
		if pol, ok := m.ProviderPolicy(provider); ok {
			if assumed, ok := parseWhenUnknown(pol.WhenUnknown); ok {
				cap.AssumedHealth, cap.EffectiveHealth = assumed, assumed
			}
		}
	}
	return cap
}

// parseWhenUnknown reads the matrix's `providers[].when_unknown`. An
// unrecognised value leaves the health UNKNOWN rather than defaulting to
// anything — a misspelled `when_unkonwn: green` must not be the reason a
// provider reads healthy.
func parseWhenUnknown(v string) (limits.Health, bool) {
	switch limits.Health(strings.ToLower(strings.TrimSpace(v))) {
	case limits.HealthGreen:
		return limits.HealthGreen, true
	case limits.HealthYellow:
		return limits.HealthYellow, true
	case limits.HealthRed:
		return limits.HealthRed, true
	case limits.HealthExhausted:
		return limits.HealthExhausted, true
	case limits.HealthUnmetered:
		return limits.HealthUnmetered, true
	}
	return "", false
}

// ---------------------------------------------------------------------------
// MODE
// ---------------------------------------------------------------------------

// ModeVerdict is the mode plus the reasoning that produced it (Invariant 5).
type ModeVerdict struct {
	Mode Mode `json:"mode"`
	// Manual is true when routing.yaml's `modes:` block named the mode outright
	// and the thresholds were not consulted.
	Manual bool     `json:"manual"`
	Reason []string `json:"reason"`
}

// DecideMode is the routing spec's §14, with §33's qualification.
//
// The manual override wins outright — `modes.providers.claude: conserve` is the
// user saying "conserve Claude for the next few hours", and a threshold that
// disagreed with it would make the control a suggestion.
//
// Otherwise:
//
//	CONSERVE   effective health is RED or EXHAUSTED, or the forecast demand
//	           before the reset exceeds what is actually left.
//	SPEND_DOWN every arm holds: the provider is GREEN overall (§33 — a healthy
//	           five-hour window does not license spending down against a
//	           constrained weekly one), some readable bucket resets within the
//	           configured window, that bucket has at least the configured
//	           remaining share, and the forecast is KNOWN and small relative to
//	           what remains.
//	NORMAL     everything else, which includes every UNKNOWN.
//
// An unknown forecast cannot satisfy either rule's demand arm, and that is the
// conservative direction on both counts: no promotion, and no phantom
// conservation.
func DecideMode(cap Capacity, demand limits.Demand, th Thresholds, manual string) ModeVerdict {
	if m, ok := ParseMode(manual); ok && m != ModeAuto {
		return ModeVerdict{Mode: m, Manual: true, Reason: []string{
			fmt.Sprintf("routing.yaml's modes: block pins %s to %s, so the thresholds were not consulted", cap.Provider, m),
		}}
	}

	v := ModeVerdict{Mode: ModeNormal}
	v.Reason = append(v.Reason, fmt.Sprintf("%s health %s (%s)", cap.Provider, cap.Health, cap.Because))
	if cap.AssumedHealth != "" {
		v.Reason = append(v.Reason, fmt.Sprintf(
			"nothing readable, so the matrix's when_unknown for %s applies: treat as %s (the reading stays UNKNOWN — the assumption is policy, not evidence)",
			cap.Provider, cap.AssumedHealth))
	}

	// CONSERVE — the scarcity arm.
	switch cap.EffectiveHealth {
	case limits.HealthRed, limits.HealthExhausted:
		v.Mode = ModeConserve
		v.Reason = append(v.Reason, fmt.Sprintf("allowance is %s, so premium capability is reserved for work where it materially matters", cap.EffectiveHealth))
		return v
	}

	tightest, haveTightest := tightestReadable(cap.Buckets)
	if demand.Known && haveTightest && tightest.RemainingPercent != nil &&
		demand.PctOfAllowance > *tightest.RemainingPercent {
		v.Mode = ModeConserve
		v.Reason = append(v.Reason, fmt.Sprintf(
			"forecast demand before the reset (%.0f%% of the allowance) exceeds the %.0f%% left on %s — %s",
			demand.PctOfAllowance, *tightest.RemainingPercent, tightest.Window, demand.Because))
		return v
	}

	// SPEND_DOWN — every arm, or nothing.
	spend := th.SpendDown
	switch {
	case cap.EffectiveHealth == limits.HealthUnmetered:
		// Not a constraint and not a shortfall: there is no expiring allowance
		// here at all, so there is nothing spend-down could convert into
		// confidence. Saying it in §33's words would be a category error.
		v.Reason = append(v.Reason, fmt.Sprintf(
			"not spending down: %s publishes no subscription allowance at all, so nothing about it expires at a reset", cap.Provider))
		return v
	case cap.EffectiveHealth != limits.HealthGreen:
		v.Reason = append(v.Reason, fmt.Sprintf(
			"not spending down: %s is %s overall, and §33 is explicit that a short window resetting soon does not license spending against a constrained or unreadable longer one",
			cap.Provider, cap.EffectiveHealth))
		return v
	case spend.TimeToResetMinutes <= 0 || spend.MinRemainingPct <= 0:
		v.Reason = append(v.Reason, "not spending down: routing.yaml's thresholds.spend_down is not configured with a usable window and floor")
		return v
	}

	near, ok := nearestReset(cap.Buckets, time.Duration(spend.TimeToResetMinutes)*time.Minute)
	if !ok {
		v.Reason = append(v.Reason, fmt.Sprintf(
			"not spending down: no readable window resets within %.0f minutes (a window whose reset has PASSED has no time-to-reset at all here, which is the arm the naive `< %.0f minutes` comparison gets wrong)",
			spend.TimeToResetMinutes, spend.TimeToResetMinutes))
		return v
	}
	if near.RemainingPercent == nil || *near.RemainingPercent < spend.MinRemainingPct {
		left := "no readable"
		if near.RemainingPercent != nil {
			left = fmt.Sprintf("%.0f%%", *near.RemainingPercent)
		}
		v.Reason = append(v.Reason, fmt.Sprintf(
			"not spending down: %s resets in %s with %s remaining, under the %.0f%% floor",
			near.Window, resetIn(near), left, spend.MinRemainingPct))
		return v
	}
	if !demand.Known {
		v.Reason = append(v.Reason, fmt.Sprintf(
			"not spending down: %s resets in %s with %.0f%% remaining, but demand before the reset is unknown (%s) and spending down on an unknown forecast is the arbitrary token consumption Invariant 6 forbids",
			near.Window, resetIn(near), *near.RemainingPercent, demand.Because))
		return v
	}
	budget := *near.RemainingPercent * spend.MaxForecastPctOfRemaining / 100
	if demand.PctOfAllowance >= budget {
		v.Reason = append(v.Reason, fmt.Sprintf(
			"not spending down: forecast demand %.0f%% is not below %.0f%% (%.0f%% of the %.0f%% remaining on %s)",
			demand.PctOfAllowance, budget, spend.MaxForecastPctOfRemaining, *near.RemainingPercent, near.Window))
		return v
	}

	v.Mode = ModeSpendDown
	v.Reason = append(v.Reason, fmt.Sprintf(
		"%s resets in %s with %.0f%% remaining and only %.0f%% of the allowance forecast before then — capacity that expires unused is worth nothing, so it buys confidence instead",
		near.Window, resetIn(near), *near.RemainingPercent, demand.PctOfAllowance))
	return v
}

// tightestReadable is the readable bucket with the least remaining allowance —
// the one that actually binds. Buckets with no readable remaining share are
// skipped rather than counted as zero.
func tightestReadable(buckets []limits.BucketReport) (limits.BucketReport, bool) {
	var best limits.BucketReport
	found := false
	for _, b := range buckets {
		if !b.Metered || b.RemainingPercent == nil {
			continue
		}
		if !found || *b.RemainingPercent < *best.RemainingPercent {
			best, found = b, true
		}
	}
	return best, found
}

// nearestReset is the readable bucket resetting soonest, when that is within
// the configured window.
//
// ResetsInSeconds is the ONLY time-to-reset in this package and
// limits.BucketHealth fills it exclusively from Reading.TimeToReset, which
// cannot return a non-positive duration. So `*b.ResetsInSeconds <= within` is
// a comparison on a strictly positive number BY CONSTRUCTION rather than by a
// sign check somebody remembered to write — which is the whole difference
// between this implementation and the literal reading of §14.
func nearestReset(buckets []limits.BucketReport, within time.Duration) (limits.BucketReport, bool) {
	var best limits.BucketReport
	found := false
	for _, b := range buckets {
		if !b.Metered || b.ResetsInSeconds == nil {
			continue
		}
		if time.Duration(*b.ResetsInSeconds)*time.Second > within {
			continue
		}
		if !found || *b.ResetsInSeconds < *best.ResetsInSeconds {
			best, found = b, true
		}
	}
	return best, found
}

func resetIn(b limits.BucketReport) string {
	if b.ResetsInSeconds == nil {
		return "an unknown time"
	}
	return (time.Duration(*b.ResetsInSeconds) * time.Second).Round(time.Minute).String()
}

// ---------------------------------------------------------------------------
// SELECTION
// ---------------------------------------------------------------------------

// Request is routing.select's input: the spec's §39 shape, plus the three
// additions the codebase forces (a cwd, because risk and the ceiling are
// per-project; an account, because health is per-account and not per-provider;
// and an explicit provider constraint, because a caller frequently wants a
// decision ABOUT a named provider rather than a free choice).
type Request struct {
	TicketID string `json:"ticketId,omitempty"`
	Role     string `json:"role"`
	// Difficulty, Risk and DecisionDensity are the caller's classification.
	// Deriving them here is §16 and is deliberately a later feature: automatic
	// classification is a model call, not a threshold.
	Difficulty      string `json:"difficulty,omitempty"`
	Risk            string `json:"risk,omitempty"`
	DecisionDensity string `json:"decisionDensity,omitempty"`

	PreviousProvider         string `json:"previousProvider,omitempty"`
	RequireIndependentFamily bool   `json:"requireIndependentFamily,omitempty"`

	// Profile names the profile to resolve under. Empty (or unknown) uses the
	// matrix's active profile.
	Profile string `json:"profile,omitempty"`
	// Provider and PreferredProvider are the same constraint under two names,
	// because both spellings are in use on the wire. Either one names the
	// provider the decision is ABOUT.
	Provider          string `json:"provider,omitempty"`
	PreferredProvider string `json:"preferredProvider,omitempty"`

	Account   string `json:"account,omitempty"`
	ProfileID string `json:"profileId,omitempty"`
	Cwd       string `json:"cwd,omitempty"`
	// CanonicalCwd is Cwd symlink-resolved, and it is `json:"-"` on purpose: no
	// caller supplies it. The HANDLER fills it in using the same canonicalizing
	// walk the spawn gate uses (bus.CanonicalizeRoot), because the ceiling lookup
	// is a LEXICAL ancestor match and a ceiling looked up on the caller's
	// spelling is a ceiling a symlink walks around. Empty — an unresolvable or
	// unnamed directory — selects the `default` ceiling, exactly as it does at
	// the gate. Two canonicalizers that could disagree would put routing.select
	// and the spawn gate back into the contradiction this field exists to end.
	CanonicalCwd string `json:"-"`

	// ForecastDemandBeforeResetPct is the caller's own estimate of how much of
	// the allowance the work ahead will consume. A POINTER because 0 is a real
	// forecast ("nothing more is coming, spend it down") and is exactly the
	// value that makes the spend-down arm fire — it must never be reachable by
	// omitting the field.
	ForecastDemandBeforeResetPct *float64 `json:"forecastDemandBeforeResetPct,omitempty"`
	// ExpectedWork is §15's phase counts, weighted by the matrix's
	// forecast_weights. See limits/forecast.go for why weighted units do not
	// become a percentage yet.
	ExpectedWork []limits.Work `json:"expectedWork,omitempty"`
}

// wantedProvider is the provider constraint, normalized. Empty means "the
// matrix chooses".
func (r Request) wantedProvider() string {
	for _, v := range []string{r.Provider, r.PreferredProvider} {
		if p := normalizeProvider(v); p != "" {
			return p
		}
	}
	return ""
}

// wantedAccount is the account the spawn will bill to. profileId is a Claude
// profile directory and is the account key the usage document uses for a
// non-default login, so it is accepted as the account when `account` is absent.
func (r Request) wantedAccount() string {
	if a := strings.TrimSpace(r.Account); a != "" {
		return a
	}
	return strings.TrimSpace(r.ProfileID)
}

// Decision is routing.select's answer.
//
// It carries the §39 fields, the capacity picture it was made against, and the
// reason list Invariant 5 requires. `Eligible` is false when the matrix cannot
// serve the requested capability on the requested provider at all — the spec's
// §32 says never silently substitute a weaker model for a provider that is
// unavailable, and "you asked for copilot and no profile puts any model of this
// capability on copilot" is the same shape of answer.
type Decision struct {
	// DecisionID is the join key: the decision log records it, the
	// routing.decision event carries it, and a spawn that acts on this answer
	// quotes it back on the wire as `decisionId`. Stamped by the HANDLER, not
	// here — Select is pure and a random id read inside it would make the policy
	// layer non-deterministic for no gain.
	DecisionID string `json:"decisionId,omitempty"`

	TicketID string `json:"ticketId,omitempty"`
	Role     string `json:"role"`
	// Capability is the capability actually selected, after any mode shift.
	Capability string `json:"capability"`
	// BaseCapability is what the role asks for before the mode moved it.
	BaseCapability string `json:"baseCapability"`
	Profile        string `json:"profile"`

	Provider string `json:"provider"`
	Model    string `json:"model,omitempty"`
	Effort   string `json:"effort,omitempty"`
	// Fresh says the worker must not inherit the previous agent's conversation
	// — what makes a same-family reviewer an actual reviewer (§6, §23).
	Fresh bool `json:"fresh,omitempty"`
	// IndependentFamily reports whether the selection is a different model
	// family from PreviousProvider. Reported even when it was not required, and
	// reported HONESTLY when it was required and could not be arranged.
	IndependentFamily bool `json:"independentFamily"`

	// Eligible is false when nothing spawnable was found; Model is then empty
	// and Reason says why.
	Eligible bool `json:"eligible"`

	// Ceiling is the directory ceiling this answer was resolved UNDER, and its
	// presence is what stops routing.select from advising something the spawn
	// gate will refuse. Nil when no ceiling row governs the target directory.
	//
	// It is reported whether or not it bit: "your project is capped at frontier
	// and this answer is under it" is as useful as the capping itself, and a
	// field that appeared only on the refusal path would leave a caller unable to
	// tell an uncapped directory from a lucky one.
	Ceiling *CeilingVerdict `json:"ceiling,omitempty"`

	Mode       Mode          `json:"mode"`
	ModeManual bool          `json:"modeManual"`
	Capacity   Capacity      `json:"capacity"`
	Demand     limits.Demand `json:"demand"`

	// ModeProvider names the provider whose capacity actually produced Mode —
	// the provider Capacity describes. It exists because Provider is the
	// provider that will RUN the work, and a mode shift is allowed to move
	// those apart (routing away from a constrained provider is the whole
	// feature). An explanation that named only one of them could claim a
	// capacity reason it never used.
	ModeProvider string `json:"modeProvider,omitempty"`
	// ShiftCapacity is the LANDING provider's own capacity, read when a mode
	// shift would have moved this role onto a provider other than ModeProvider.
	// It is present whether the shift was applied or refused: "we looked at
	// claude before sending claude the work" is the claim, and a field that
	// appeared only on success would not support it.
	ShiftCapacity *Capacity `json:"shiftCapacity,omitempty"`
	// ShiftMode is the mode that landing provider's OWN capacity decides, which
	// is what the shift was allowed or refused on.
	ShiftMode Mode `json:"shiftMode,omitempty"`

	Reason []string `json:"reason"`

	// Matrix names where the answer came from, so an operator can tell a
	// hand-edited file's answer from the compiled-in default's.
	Matrix MatrixInfo `json:"matrix"`
	// FellBack is set when the user's document could not answer and the shipped
	// default did.
	FellBack bool `json:"fellBack,omitempty"`
	// DecidedAt is the instant the currency verdicts were reached.
	DecidedAt int64 `json:"decidedAt"`
}

// MatrixInfo is the provenance of the matrix behind a decision.
type MatrixInfo struct {
	Source  string `json:"source,omitempty"`
	Version int    `json:"version"`
	// Issues is how many load-time findings the running matrix carries. A
	// decision made against a matrix with unresolved issues is still a
	// decision, and the count is how a reader learns to go and look.
	Issues int `json:"issues"`
}

// Select is the whole of the spec's §30, in the order §30 gives.
//
// The order matters and is not arbitrary: the provider is fixed BEFORE the mode
// is decided (health is per-provider), and the mode is decided BEFORE the
// capability is finalized (the mode is what moves it). Doing it the other way
// round produces a mode computed for one provider and applied to another, which
// is the incoherence the harness's copilot case would not catch and a real
// fleet would.
func Select(m *Matrix, snap limits.Snapshot, snapErr error, now time.Time, req Request) Decision {
	d := Decision{
		TicketID:  strings.TrimSpace(req.TicketID),
		Role:      strings.TrimSpace(req.Role),
		DecidedAt: now.Unix(),
		Mode:      ModeNormal,
	}
	if m == nil {
		d.Reason = []string{"no routing matrix is loaded at all — this hub cannot answer a routing question"}
		return d
	}
	d.Matrix = MatrixInfo{Source: m.Source, Version: m.Version, Issues: len(m.Issues)}

	// 1. Profile.
	profile, fellBack := m.ActiveProfileName()
	if named := strings.TrimSpace(req.Profile); named != "" {
		if _, ok := m.Profiles[named]; ok {
			profile = named
		} else {
			d.Reason = append(d.Reason, fmt.Sprintf("the request names profile %q, which this matrix does not define — using %q", named, profile))
		}
	}
	d.Profile = profile
	d.FellBack = fellBack

	// 2. Role -> capability.
	resolved, err := m.ResolveRole(d.Role)
	if err != nil {
		d.Reason = append(d.Reason, fmt.Sprintf("no capability for role %q: %v", d.Role, err))
		return d
	}
	d.BaseCapability, d.Capability = resolved.Capability, resolved.Capability
	d.FellBack = d.FellBack || resolved.FellBack
	d.Reason = append(d.Reason, fmt.Sprintf("role %s requires capability %s under profile %s", d.Role, d.BaseCapability, profile))

	// 3. The provider the decision is ABOUT. A named one is honoured as the
	//    subject whether or not the matrix can serve the capability on it,
	//    because the caller asked about that provider's capacity and an answer
	//    describing a different one is not an answer.
	subject := req.wantedProvider()
	constrained := subject != ""
	if !constrained {
		if a, err := m.ResolveCapability(profile, d.BaseCapability); err == nil {
			subject = a.Provider
		}
	}
	if subject == "" {
		d.Reason = append(d.Reason, fmt.Sprintf("profile %s does not resolve capability %s to any provider", profile, d.BaseCapability))
		return d
	}
	d.Provider = subject
	if constrained {
		d.Reason = append(d.Reason, fmt.Sprintf("the request asks about provider %s, so its capacity is what governs this decision", subject))
	}

	// 3b. PROVIDER-LEVEL DISABLEMENT, before any capacity is read or any
	//     assignment is resolved. routing.yaml documents `enabled: false` as the
	//     one spelling for taking something out of service — a deep merge cannot
	//     delete, so an operator who wants codex gone writes that line and
	//     expects it to mean something. Honouring it only on the per-capability
	//     ASSIGNMENT (step 8) would leave the provider-level flag parsed,
	//     reported and never enforced, which looks identical to working.
	if reason, off := providerDisabled(m, subject); off {
		d.Reason = append(d.Reason, reason)
		return d
	}

	// 4. Capacity, judged against THIS instant.
	d.Capacity = capacityFor(m, snap, snapErr, subject, req.wantedAccount(), now)

	// 5. Forecast, from the matrix's own weights.
	d.Demand = limits.Forecast(req.ForecastDemandBeforeResetPct, req.ExpectedWork, m.ForecastWeights)

	// 6. Mode, from the SUBJECT provider's capacity. d.ModeProvider records
	//    whose capacity that was, because step 7 is allowed to hand the work to
	//    somebody else.
	verdict := DecideMode(d.Capacity, d.Demand, m.Thresholds, m.ModeFor(subject))
	d.Mode, d.ModeManual, d.ModeProvider = verdict.Mode, verdict.Manual, subject
	d.Reason = append(d.Reason, verdict.Reason...)

	// 7. The mode moves the capability, if the matrix says it does — and if the
	//    provider the move LANDS ON can take it.
	//
	// A SHIFT MAY CROSS PROVIDERS, DELIBERATELY. That is the feature: §12 says
	// conserve "should also shift workload toward another healthy provider",
	// and the shipped `mixed` profile already puts the reviewer capabilities on
	// claude while the implementer ones are codex. Constraining the shift to
	// the evaluated provider would delete the one thing limit-aware routing is
	// for.
	//
	// What is NOT allowed is deciding on one provider's capacity and spending
	// another's without ever looking at it. So when the shift crosses, the
	// landing provider's capacity is READ AND JUDGED IN ITS OWN RIGHT, and the
	// move is refused if that provider is itself conserving: a shift onto a red
	// provider because a green one was constrained is worse than not shifting,
	// and it is worse in exactly the direction this feature exists to prevent.
	if shifted, ok := m.ShiftFor(string(d.Mode), d.Role); ok && shifted != d.Capability {
		d.applyShift(m, snap, snapErr, now, req, profile, subject, constrained, shifted)
	}

	// 7b. THE CEILING, APPLIED HERE RATHER THAN LEFT TO THE GATE.
	//
	// The spawn gate clamps every bus agents.spawn to the ceiling configured for
	// its directory. Until this step existed, routing.select did not know that:
	// it would answer "Fable for the judge" in a directory capped at frontier,
	// the gate would take the model away, and the dispatch arrived as an
	// unexplained downgrade — a system contradicting itself once per judge.
	//
	// The tempting fix was to let a caller relay a decision id and have the gate
	// stand aside for it. That is unsound: `decisionId` is caller-supplied,
	// published on an open-by-decision event, forgeable and replayable, and a
	// ceiling a caller can talk past by asserting the system told it to is worse
	// than no ceiling, because it reads as protective. The contradiction is
	// removed at the SOURCE instead — the advice is capped, so there is nothing
	// for the gate to disagree with. The gate keeps clamping regardless: it is
	// the security boundary and it must still refuse a caller that ignored
	// routing entirely. Belt and braces.
	//
	// The SAME function the gate calls (CheckSpawn) answers here, against the
	// same canonical directory, so "capped identically" is a property of there
	// being one implementation rather than of two staying in agreement.
	if v := m.CheckSpawn(SpawnRequest{
		CanonicalCwd:           req.CanonicalCwd,
		Capability:             d.Capability,
		Provider:               subject,
		SkipReplacementRouting: true, // step 8 below resolves the capped capability itself
	}); v.Key != "" {
		d.Ceiling = &v
		if v.Denied {
			d.Reason = append(d.Reason, v.Because...)
			d.Reason = append(d.Reason, "the ceiling governing this directory cannot be read, so there is no answer that is safe to act on — routing REFUSES rather than advising something the spawn gate would also refuse")
			return d
		}
		if v.CapabilityRefused {
			d.Reason = append(d.Reason, v.Because...)
			d.Capability = v.Capability
		}
	}

	// 8. Resolve the capability to something spawnable, honouring the provider
	//    constraint and the independence preference.
	a, from, ok := m.assignmentFor(profile, d.Capability, subject, constrained)
	if !ok {
		d.Reason = append(d.Reason, fmt.Sprintf(
			"no profile in this matrix resolves capability %s to provider %s, so there is nothing to spawn there — asking again without the provider constraint, or adding the pairing to routing.yaml, are the two honest fixes. A weaker model on a provider you did not ask for is not one (§32)",
			d.Capability, subject))
		return d
	}
	if from != profile {
		d.Reason = append(d.Reason, fmt.Sprintf("profile %s does not put %s on %s; took the pairing from profile %s", profile, d.Capability, subject, from))
	}
	if !a.IsEnabled() {
		d.Reason = append(d.Reason, fmt.Sprintf("profile %s has capability %s explicitly disabled (enabled: false)", from, d.Capability))
		return d
	}
	// The final assignment's provider, checked again on the way out. Step 3b
	// covered the subject; this covers the provider a cross-profile or
	// cross-provider resolution actually landed on, so there is no arm of this
	// function that can return an eligible decision naming a disabled provider.
	if reason, off := providerDisabled(m, a.Provider); off {
		d.Reason = append(d.Reason, reason)
		return d
	}

	d.Provider, d.Model, d.Effort, d.Fresh = a.Provider, a.Model, a.Effort, a.Fresh
	d.Eligible = true

	prev := normalizeProvider(req.PreviousProvider)
	d.IndependentFamily = prev == "" || prev != d.Provider
	if req.RequireIndependentFamily && !d.IndependentFamily {
		// Reported, never silently ignored: §23 makes independence part of the
		// requirement rather than a coincidence, and a reviewer that inherited
		// the implementer's family without anyone saying so is the failure.
		d.Reason = append(d.Reason, fmt.Sprintf(
			"independent family was REQUIRED and could not be arranged: %s is the only provider this matrix puts %s on, and it also ran the previous agent — set `fresh: true` on that entry, or add a cross-family pairing",
			d.Provider, d.Capability))
	}
	if prev != "" {
		d.Reason = append(d.Reason, fmt.Sprintf("previous agent ran on %s; this one runs on %s", prev, d.Provider))
	}

	d.Reason = append(d.Reason, fmt.Sprintf("selected %s %s%s for capability %s under profile %s",
		d.Provider, d.Model, effortSuffix(d.Effort), d.Capability, from))
	return d
}

// providerDisabled reports whether routing.yaml takes this provider out of
// service outright, with the sentence a refusal quotes.
//
// A provider with no `providers:` entry at all is NOT disabled: the block
// describes capacity, and a matrix that named a provider only under `profiles:`
// must still be able to route there.
func providerDisabled(m *Matrix, provider string) (string, bool) {
	pol, ok := m.ProviderPolicy(provider)
	if !ok || pol.IsEnabled() {
		return "", false
	}
	return fmt.Sprintf(
		"routing.yaml's providers.%s is explicitly disabled (enabled: false), so nothing routes there — that flag is the ONE spelling for taking a provider out of service (a deep merge cannot delete), and honouring it anywhere but here would make it a comment. Re-enable it, or ask for a provider that is in service",
		provider), true
}

// capacityFor is step 4's reading, factored out because step 7 needs the same
// reading for a DIFFERENT provider — the one a mode shift would land on.
//
// A usage report that could not be read is UNKNOWN capacity with the failure
// named, never an error and never a healthy default.
func capacityFor(m *Matrix, snap limits.Snapshot, snapErr error, provider, account string, now time.Time) Capacity {
	if snapErr == nil {
		return ReadCapacity(m, snap, provider, account, now)
	}
	cap := Capacity{
		Provider: provider,
		Health:   limits.HealthUnknown,
		Because:  fmt.Sprintf("claudemon's usage report could not be read (%v), so no provider's capacity is knowable right now", snapErr),
	}
	cap.EffectiveHealth = cap.Health
	if pol, ok := m.ProviderPolicy(provider); ok {
		if assumed, ok := parseWhenUnknown(pol.WhenUnknown); ok {
			cap.AssumedHealth, cap.EffectiveHealth = assumed, assumed
		}
	}
	return cap
}

// applyShift is step 7: move the role's capability if the matrix says the mode
// moves it, AND the provider that move lands on can actually take the work.
//
// The check that matters is the cross-provider one. The mode was computed from
// ModeProvider's capacity; if the shifted capability resolves somewhere else,
// that second provider's capacity has never been read, and applying the move
// would be a decision justified by a reading of a machine that is not going to
// run the work. So it is read, judged by the same §14 rules and the same
// matrix (including that provider's own `modes:` override), and:
//
//   - CONSERVE there means the landing provider is itself scarce or
//     over-forecast. The shift is REFUSED and the role keeps its ordinary
//     capability on the subject. Not shifting is the smaller mistake.
//   - `enabled: false` there means the same refusal for a different reason.
//   - anything else applies the shift, and the decision then reports BOTH
//     capacities, so the sentence "we moved this to claude because codex is
//     red" is backed by a claude reading rather than by an assumption.
func (d *Decision) applyShift(
	m *Matrix, snap limits.Snapshot, snapErr error, now time.Time, req Request,
	profile, subject string, constrained bool, shifted string,
) {
	from := d.Capability
	move := fmt.Sprintf("routing.yaml's mode_shifts moves role %s from %s to %s under %s", d.Role, from, shifted, d.Mode)

	a, _, ok := m.assignmentFor(profile, shifted, subject, constrained)
	if !ok {
		d.Reason = append(d.Reason, fmt.Sprintf(
			"%s — but nothing in this matrix resolves %s to something spawnable on %s, so the role keeps %s rather than being left with no assignment at all",
			move, shifted, subject, from))
		return
	}
	landing := normalizeProvider(a.Provider)
	if landing == "" || landing == subject {
		d.Reason = append(d.Reason, fmt.Sprintf("%s, on %s — the same provider whose capacity decided the mode", move, subject))
		d.Capability = shifted
		return
	}

	// The move crosses providers. Read the one that will actually run it.
	landCap := capacityFor(m, snap, snapErr, landing, req.wantedAccount(), now)
	landVerdict := DecideMode(landCap, d.Demand, m.Thresholds, m.ModeFor(landing))
	d.ShiftCapacity, d.ShiftMode = &landCap, landVerdict.Mode

	if reason, off := providerDisabled(m, landing); off {
		d.Reason = append(d.Reason, fmt.Sprintf(
			"%s — REFUSED, because it lands on %s and %s", move, landing, reason))
		return
	}
	if landVerdict.Mode == ModeConserve {
		d.Reason = append(d.Reason, fmt.Sprintf(
			"%s — REFUSED. The move lands on %s, not on %s, so %s's own capacity was read rather than assumed: %s (%s), which is itself CONSERVE. Moving work onto a constrained provider because a different one was constrained is worse than not moving it, so role %s keeps %s on %s",
			move, landing, subject, landing, landCap.EffectiveHealth, landCap.Because, d.Role, from, subject))
		return
	}
	d.Reason = append(d.Reason, fmt.Sprintf(
		"%s — and the move lands on %s rather than on %s, so %s's OWN capacity was read before applying it: %s (%s), mode %s. The mode came from %s's capacity; the work goes to %s, whose capacity supports it",
		move, landing, subject, landing, landCap.EffectiveHealth, landCap.Because, landVerdict.Mode, subject, landing))
	d.Capability = shifted
}

func effortSuffix(effort string) string {
	if effort == "" {
		return ""
	}
	return " (effort " + effort + ")"
}

// assignmentFor resolves a capability to an assignment, honouring a provider
// constraint by looking beyond the active profile when it has to.
//
// When the caller named no provider, the active profile answers and that is the
// end of it. When the caller DID name one, the active profile is tried first
// and every other profile after it, in name order for determinism — a matrix
// that puts `frontier` on codex under `mixed` and on claude under
// `anthropic_only` can answer "frontier, on claude" without the caller having
// to know which profile spells it that way.
func (m *Matrix) assignmentFor(profile, capability, provider string, constrained bool) (Assignment, string, bool) {
	if !constrained {
		a, err := m.ResolveCapability(profile, capability)
		if err != nil {
			return Assignment{}, "", false
		}
		return a, profile, true
	}
	if a, err := m.ResolveCapability(profile, capability); err == nil && a.Provider == provider {
		return a, profile, true
	}
	names := make([]string, 0, len(m.Profiles))
	for name := range m.Profiles {
		if name != profile {
			names = append(names, name)
		}
	}
	sort.Strings(names)
	for _, name := range names {
		if a, err := m.ResolveCapability(name, capability); err == nil && a.Provider == provider {
			return a, name, true
		}
	}
	return Assignment{}, "", false
}
