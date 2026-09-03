package routing

import (
	"encoding/json"
	"strings"
	"testing"
)

// ---------------------------------------------------------------------------
// LIVE PROVIDER AVAILABILITY
//
// Every case here uses the shipped `mixed` implementer — primary codex, first
// alternative claude — with BOTH providers green, so health cannot explain any
// of the answers and availability is the only thing moving them.
// ---------------------------------------------------------------------------

func availImplementer(t *testing.T, avail ProviderAvailability) Decision {
	t.Helper()
	zero := 0.0
	return Select(shipped(t), altSnapshot(t, altGreen, altGreen), nil, avail, policyNow, Request{
		Role: "implementer", ForecastDemandBeforeResetPct: &zero,
	})
}

// TestAnUnavailableProviderFallsOverWithTheReasonNamed is the feature: a
// provider whose CLI cannot launch anything is not a routing target, however
// healthy its allowance looks.
func TestAnUnavailableProviderFallsOverWithTheReasonNamed(t *testing.T) {
	d := availImplementer(t, ProviderAvailability{
		"codex": {Available: false, Reason: "codex answered the model catalog with no launchable model"},
	})
	if !d.Eligible || d.Provider != "claude" || d.Model != "opus" {
		t.Fatalf("got %+v, want the claude alternative — a green allowance on a CLI that can launch nothing is not a place to send work", d)
	}
	if d.FellOverFrom == nil || d.FellOverFrom.Provider != "codex" {
		t.Fatalf("FellOverFrom = %+v, want the codex primary this answer passed over", d.FellOverFrom)
	}
	joined := strings.Join(d.Reason, " ")
	if !strings.Contains(joined, "codex is not available to launch right now") {
		t.Errorf("the walk does not say WHY it moved: %v", d.Reason)
	}
	if !strings.Contains(joined, "no launchable model") {
		t.Errorf("the probe's own reason is not quoted, so an operator cannot tell a CLI that launches nothing from a dead daemon: %v", d.Reason)
	}
	if d.Capability != "frontier" {
		t.Errorf("capability = %q — availability changes the PROVIDER, never the tier of work", d.Capability)
	}
}

// TestAMissingAvailabilityEntryFailsOpen is the safety rule, and it is asserted
// as byte-for-byte agreement with the pre-availability answer: "we could not
// ask" must route exactly as it always did.
//
// Without it, a hub that cannot reach claudemon for thirty seconds would
// declare every provider dead and route nowhere — which is worse than routing
// to a provider that turns out to be missing, because that failure is loud and
// immediate and this one looks like the router breaking for no reason.
func TestAMissingAvailabilityEntryFailsOpen(t *testing.T) {
	none := availImplementer(t, nil)
	if !none.Eligible || none.Provider != "codex" {
		t.Fatalf("got %+v, want the ordinary codex primary with no availability map at all", none)
	}
	for _, name := range []string{"an empty map", "a map that names another provider", "a map that names codex as available"} {
		var avail ProviderAvailability
		switch name {
		case "an empty map":
			avail = ProviderAvailability{}
		case "a map that names another provider":
			avail = ProviderAvailability{"copilot": {Available: false, Reason: "no copilot CLI here"}}
		default:
			avail = ProviderAvailability{"codex": {Available: true}}
		}
		got, err := json.Marshal(availImplementer(t, avail))
		if err != nil {
			t.Fatal(err)
		}
		want, err := json.Marshal(none)
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != string(want) {
			t.Errorf("%s changed the answer\n got: %s\nwant: %s", name, got, want)
		}
	}

	// The unit underneath, directly: an unknown provider is not unusable, and a
	// nil map answers for every provider at once.
	if why, off := (ProviderAvailability)(nil).Unusable("codex"); off {
		t.Errorf("a nil availability map reported codex unusable: %q", why)
	}
	if why, off := (ProviderAvailability{"claude": {Available: false}}).Unusable("codex"); off {
		t.Errorf("a provider absent from the map reported unusable: %q", why)
	}
	// …and an unavailable entry with no reason still explains itself.
	why, off := (ProviderAvailability{"codex": {Available: false}}).Unusable("openai")
	if !off {
		t.Fatal("an unavailable provider was not reported unusable through its vendor alias")
	}
	if !strings.Contains(why, "no launchable models") {
		t.Errorf("a refusal with no quotable reason must still say something an operator can act on, got %q", why)
	}
}

// TestAModelFlaggedAtLoadIsStillUnusableOnAnAvailableProvider keeps slice 1's
// rule intact: availability is about the PROVIDER, and a row the loader already
// condemned is still not a row to route to.
func TestAModelFlaggedAtLoadIsStillUnusableOnAnAvailableProvider(t *testing.T) {
	m := shipped(t)
	cat := &fakeCatalog{models: map[string][]CatalogModel{
		// codex is up and answering — it just does not serve the model the
		// matrix names for `frontier`.
		"codex":  {{ID: "gpt-5.6-luna"}, {ID: "gpt-5.6-terra"}},
		"claude": {{ID: "opus"}, {ID: "sonnet"}, {ID: "fable"}},
	}}
	m.Issues = append(m.Issues, ValidateAgainstCatalog(m, cat)...)
	// The fixture just did what Service.ValidateCatalog does on a real check —
	// say so, or the walk reads this matrix as still awaiting one and adds a
	// caveat this test does not expect.
	m.CatalogChecked = true
	if !hasIssueAt(m.Issues, "profiles.mixed.frontier") {
		t.Fatalf("fixture drift: the catalog check did not flag the frontier primary: %v", m.Issues)
	}

	zero := 0.0
	d := Select(m, altSnapshot(t, altGreen, altGreen), nil,
		ProviderAvailability{"codex": {Available: true}, "claude": {Available: true}},
		policyNow, Request{Role: "implementer", ForecastDemandBeforeResetPct: &zero})
	if d.Provider != "claude" {
		t.Fatalf("got %s — a live provider whose specific MODEL the loader flagged is still not routable", d.Provider)
	}
	if !strings.Contains(strings.Join(d.Reason, " "), "matrix's validation flags profiles.mixed.frontier") {
		t.Errorf("the answer quotes the wrong reason for passing the primary over: %v", d.Reason)
	}
}

// TestAvailabilityIsReadBeforeHealth pins the ORDER in unusable(): a provider
// that cannot be started is not usefully described by its allowance, and the
// reason an operator acts on is the one that names the CLI.
func TestAvailabilityIsReadBeforeHealth(t *testing.T) {
	zero := 0.0
	// codex is BOTH red and unavailable. Only one sentence can be the reason
	// the walk quotes, and it has to be the actionable one.
	d := Select(shipped(t), altSnapshot(t, altGreen, altRed), nil,
		ProviderAvailability{"codex": {Available: false, Reason: "no codex CLI on this machine"}},
		policyNow, Request{Role: "implementer", ForecastDemandBeforeResetPct: &zero})
	joined := strings.Join(d.Reason, " ")
	if !strings.Contains(joined, "no codex CLI on this machine") {
		t.Errorf("the walk quoted the allowance instead of the missing CLI: %v", d.Reason)
	}
	if strings.Contains(joined, "primary codex gpt-5.6-sol unusable (codex's allowance is red") {
		t.Errorf("health answered first for a provider that cannot be started at all: %v", d.Reason)
	}
}
