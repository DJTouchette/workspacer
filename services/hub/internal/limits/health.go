package limits

// The health ladder: the routing spec's §9, over the buckets bucket.go
// produces, with the thresholds supplied by the caller rather than written
// here.
//
// THIS FILE MAKES NO POLICY DECISION. It says how much of an allowance is gone
// and whether anyone can tell; what to DO about a constrained or an unreadable
// provider is internal/routing's question and is answered there, from the
// matrix. The thresholds arrive as a Bands value because they live in
// routing.yaml under `thresholds.health:` — 70 and 90 are the user's numbers
// and must not become constants in Go.
//
// TWO RULES CARRY OVER FROM THE CURRENCY GUARD AND ARE THE POINT OF THE FILE:
//
//  1. Nothing here reads a percentage except through Reading.UsedPercent, which
//     refuses one off a window that has closed. There is no arm below that can
//     see the stale 67% the whole feature exists because of.
//  2. UNKNOWN IS AN ANSWER AND IT IS NOT GREEN. An unreadable bucket never
//     folds into a healthy provider: in [Worst] it outranks GREEN, so a
//     provider whose short window cannot be read is UNKNOWN even when its
//     weekly window is comfortable. Three of five providers have no readable
//     quota permanently — copilot answers 403 and opencode/pi never appear in
//     the document at all — so the difference between "nothing to conserve"
//     (UNMETERED) and "we cannot tell" (UNKNOWN) is load-bearing, not
//     pedantic, and both are modelled.

import (
	"fmt"
	"sort"
)

// Health is the fleet-facing capacity state of a bucket or a provider.
//
// The spec's §9 names five. There are six here, and the sixth is the one the
// data forces: UNMETERED. A provider that publishes no allowance at all is not
// in the dark about a number it has — there is no number, permanently, and
// conserving it conserves nothing. Collapsing that into UNKNOWN makes copilot
// look like a provider having a bad day forever; collapsing it into GREEN
// claims capacity nobody measured. The routing matrix already distinguishes the
// two through `providers[].when_unknown`, so the ladder does too.
type Health string

const (
	// HealthGreen is normal capacity. No conservation required.
	HealthGreen Health = "green"
	// HealthYellow means the allowance is becoming constrained. Premium usage
	// should become more selective.
	HealthYellow Health = "yellow"
	// HealthRed means the allowance is scarce. Reserve the top capabilities for
	// work where they materially matter.
	HealthRed Health = "red"
	// HealthExhausted means the allowance is spent: route around it.
	HealthExhausted Health = "exhausted"
	// HealthUnknown means there is no trustworthy limit information. It is NOT
	// a synonym for healthy and it is not a synonym for unmetered: something
	// could be known here and currently is not.
	HealthUnknown Health = "unknown"
	// HealthUnmetered means the question does not apply — this provider
	// publishes no subscription allowance at all, so there is nothing to
	// conserve and nothing to spend down.
	HealthUnmetered Health = "unmetered"
)

// severity orders the ladder for [Worst]. UNKNOWN sits ABOVE green and BELOW
// yellow, which is the whole of rule 2: a definite constraint dominates an
// unreadable one (a red weekly window is worse news than an unreadable
// five-hour one), and an unreadable one dominates a clean bill of health (a
// green window must never mask a window nobody could read). UNMETERED sits at
// the bottom because it is not a reading at all — a provider with one metered
// bucket and two unmetered ones is described by the metered one.
var severity = map[Health]int{
	HealthUnmetered: 0,
	HealthGreen:     1,
	HealthUnknown:   2,
	HealthYellow:    3,
	HealthRed:       4,
	HealthExhausted: 5,
}

// Bands are the used-percentage thresholds the ladder steps at. They come from
// the routing matrix (`thresholds.health.yellow_at_used_pct` /
// `red_at_used_pct`), never from here.
type Bands struct {
	YellowAtUsedPct float64
	RedAtUsedPct    float64
}

// Valid reports whether these bands can order anything. A zeroed or inverted
// pair is a configuration failure, not a reason to invent numbers: [BucketHealth]
// answers UNKNOWN rather than judging every provider GREEN against a 0/0 ladder.
func (b Bands) Valid() bool {
	return b.YellowAtUsedPct > 0 && b.RedAtUsedPct > 0 && b.RedAtUsedPct >= b.YellowAtUsedPct
}

// BucketReport is one bucket's verdict with the sentence that justifies it.
//
// The sentence is not decoration. A provider that silently vanishes from a
// routing decision is indistinguishable from one that was considered and
// rejected, and this is the cheapest detection the feature has (spec §31,
// Invariant 5).
type BucketReport struct {
	ID     string `json:"id"`
	Window string `json:"window"`
	Health Health `json:"health"`
	// Metered is false for a window this provider will never publish. Such a
	// bucket is excluded from the provider fold entirely; see [Worst].
	Metered bool `json:"metered"`
	// UsedPercent / RemainingPercent are present ONLY on a current reading with
	// a readable utilization — the currency guard's terms, unchanged.
	UsedPercent      *float64 `json:"usedPercent,omitempty"`
	RemainingPercent *float64 `json:"remainingPercent,omitempty"`
	// ResetsInSeconds is present only when a strictly positive time-to-reset
	// exists. There is no arm here that can produce a zero or a negative one.
	ResetsInSeconds *int64 `json:"resetsInSeconds,omitempty"`
	// State is the currency verdict — current, rolled-over or unreadable.
	State  WindowState `json:"state"`
	Reason string      `json:"reason,omitempty"`
	// Explain is the human sentence, including what the last raw reading said
	// when the window it described has since closed.
	Explain string `json:"explain"`
	// Source is the wire's provenance word for the account row.
	Source string `json:"source,omitempty"`
}

// BucketHealth judges ONE bucket against the bands.
//
// The order of the arms is the invariant: currency first, thresholds second.
// Every number below arrives through Reading, which refuses to produce one off
// a window that has closed, so there is no comparison here that a stale reading
// can reach.
func BucketHealth(b Bucket, bands Bands) BucketReport {
	r := BucketReport{
		ID:      b.ID(),
		Window:  b.Window,
		Metered: b.Metered(),
		State:   b.Reading.State(),
		Reason:  b.Reading.Reason(),
		Source:  b.Source,
		Explain: explainBucket(b),
	}

	switch {
	case !r.Metered:
		// Structurally unpublishable: copilot's 403, and every window a
		// provider declares it does not have. Not a dark reading — an absent
		// meter.
		r.Health = HealthUnmetered
		return r
	case !b.Reading.Usable():
		r.Health = HealthUnknown
		return r
	}

	if ttr, ok := b.Reading.TimeToReset(); ok {
		secs := int64(ttr.Seconds())
		r.ResetsInSeconds = &secs
	}

	used, ok := b.Reading.UsedPercent()
	if !ok {
		// Currency and readability are separate axes and this is the case that
		// proves it: the window IS the one running, so its reset is knowable,
		// while its utilization is not. Answering GREEN here would be inventing
		// the number the source declined to give.
		r.Health = HealthUnknown
		return r
	}
	remaining, _ := b.Reading.RemainingPercent()
	r.UsedPercent, r.RemainingPercent = &used, &remaining

	if !bands.Valid() {
		// A matrix with no usable health thresholds cannot order anything, and
		// the honest answer to "how healthy is 40%?" without a ladder is that
		// nobody said.
		r.Health = HealthUnknown
		return r
	}

	switch {
	case used >= 100:
		r.Health = HealthExhausted
	case used >= bands.RedAtUsedPct:
		r.Health = HealthRed
	case used >= bands.YellowAtUsedPct:
		r.Health = HealthYellow
	default:
		r.Health = HealthGreen
	}
	return r
}

// explainBucket is the sentence, and it is the ONE place the ungated raw
// scalar is allowed anywhere near a decision — as prose, never as a number a
// comparison can reach.
//
// "codex 5h unknown — the last reading was 67% used and its window has since
// rolled over" is strictly more useful than dropping codex silently, and it is
// what a person reads when they ask why the fleet stopped using a provider.
func explainBucket(b Bucket) string {
	base := b.Reading.Explain()
	if b.Reading.Usable() {
		return base
	}
	raw := b.DisplayOnlyRawUsedPercent()
	if v, ok := raw.Number(); ok {
		return fmt.Sprintf("%s (the last reading said %.0f%% used, and it is history, not a present capacity)", base, v)
	}
	if reason := raw.Reason; reason != "" {
		return fmt.Sprintf("%s (%s)", base, reason)
	}
	return base
}

// ProviderReport is one provider's capacity picture for ONE account, folded.
type ProviderReport struct {
	Provider string `json:"provider"`
	// Account is the account key the buckets belong to; "" is the default
	// login and is a real answer.
	Account string `json:"account,omitempty"`
	// Health is the fold. See [Worst].
	Health Health `json:"health"`
	// Metered is true when at least one bucket describes an allowance that
	// could be read. False means this provider publishes nothing, ever.
	Metered bool           `json:"metered"`
	Buckets []BucketReport `json:"buckets"`
	// Because is the one-sentence justification for Health, so a routing
	// decision can quote it.
	Because string `json:"because"`
}

// Worst folds bucket verdicts into the provider's effective health, which is
// the spec's §33: *"effective health = worst applicable meaningful limit
// bucket"*, with both qualifiers doing real work.
//
//	APPLICABLE   an unmetered bucket is skipped. Codex declares no monthly
//	             window and copilot declares none at all; folding those in as
//	             UNKNOWN would make every provider permanently unknown, which is
//	             the mirror-image mistake of folding them in as GREEN.
//	MEANINGFUL   among what is left, the worst wins — and UNKNOWN outranks GREEN,
//	             so a readable weekly window can never vouch for a five-hour one
//	             nobody could read.
//
// An empty bucket list means the provider is absent from the usage document
// entirely (opencode, pi), which is UNKNOWN with a reason — not "0% used".
func Worst(reports []BucketReport) (Health, string) {
	if len(reports) == 0 {
		return HealthUnknown, "no limit buckets at all — this provider does not appear in the usage report"
	}
	worst, worstIdx := HealthUnmetered, -1
	metered := 0
	for i, r := range reports {
		if !r.Metered {
			continue
		}
		metered++
		if severity[r.Health] > severity[worst] || worstIdx < 0 {
			worst, worstIdx = r.Health, i
		}
	}
	if metered == 0 {
		return HealthUnmetered, "every window this provider publishes is structurally unavailable, so there is no allowance to conserve"
	}
	return worst, fmt.Sprintf("worst of %d readable window(s): %s %s — %s",
		metered, reports[worstIdx].Window, reports[worstIdx].Health, reports[worstIdx].Explain)
}

// ResolveAccount picks the account row whose allowance a decision draws on.
//
// It exists because the bucket key is (provider, ACCOUNT, window) while a
// routing request may name no account at all, and the two ways of guessing are
// both wrong. Folding every account together is the lie bucket.go's header
// describes — live on 2026-08-30 one machine carried three claude rows at once,
// one healthy, one with an expired token and one unattributable. Defaulting to
// the empty string is worse, because "" is the CLAUDE DEFAULT LOGIN and a real
// answer, so an unspecified codex request would silently match nothing and read
// as "codex is not in the report".
//
// So: an explicitly named account must exist, and an unnamed one resolves to the
// provider's own default row. `known` is false when neither is available, which
// is UNKNOWN with a reason rather than a fold over rows the spawn will not bill.
func ResolveAccount(buckets []Bucket, provider, requested string) (account string, known bool) {
	var firstAttributable string
	haveFirst := false
	for _, b := range buckets {
		if b.Provider != provider || !b.AccountKnown {
			continue
		}
		if requested != "" {
			if b.Account == requested {
				return requested, true
			}
			continue
		}
		if b.IsDefault {
			return b.Account, true
		}
		if !haveFirst {
			firstAttributable, haveFirst = b.Account, true
		}
	}
	if requested == "" && haveFirst {
		// No row calls itself the default. One attributable account is still a
		// better answer than none, and it is named in the report so the caller
		// can see which one it got.
		return firstAttributable, true
	}
	return "", false
}

// ForProvider builds the fold for one provider and account out of a judged
// bucket list. `account` is matched exactly; "" is the default login.
//
// Buckets whose account the daemon could not determine are DELIBERATELY EXCLUDED
// — they describe usage nobody can attribute, and a spawn always names the
// account it will bill to, so folding them in would bill a decision to the wrong
// allowance. Snapshot.Bucket makes the same choice for the same reason.
func ForProvider(buckets []Bucket, provider, account string, bands Bands) ProviderReport {
	out := ProviderReport{Provider: provider, Account: account}
	for _, b := range buckets {
		if b.Provider != provider || !b.AccountKnown || b.Account != account {
			continue
		}
		out.Buckets = append(out.Buckets, BucketHealth(b, bands))
	}
	sort.SliceStable(out.Buckets, func(i, j int) bool {
		return windowRank(out.Buckets[i].Window) < windowRank(out.Buckets[j].Window)
	})
	for _, r := range out.Buckets {
		if r.Metered {
			out.Metered = true
			break
		}
	}
	out.Health, out.Because = Worst(out.Buckets)
	return out
}

func windowRank(window string) int {
	for i, w := range WindowOrder {
		if w == window {
			return i
		}
	}
	return len(WindowOrder)
}
