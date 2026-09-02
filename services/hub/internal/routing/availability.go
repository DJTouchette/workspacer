package routing

// LIVE PROVIDER AVAILABILITY: a fallover trigger that is a fact about right
// now, injected rather than probed.
//
// Everything the fallover walk could refuse a candidate for used to be a fact
// about the DOCUMENT — a row switched off, a load-time Issue, a health reading
// out of the usage report. alternatives.go says so in as many words: "there is
// no live probe of whether a provider's CLI is actually installed… making it
// visible is a live-detection feature and belongs where the catalog is already
// fetched, not inside a function that must not do I/O".
//
// This is that feature, built the way that sentence says. The hub already boots
// each provider's CLI to fetch its model catalog (cmd/hub/routingselect.go's
// routingCatalog, whose answers ValidateAgainstCatalog checks the matrix
// against). That probe already knows whether a provider answered at all, so the
// answer is folded into this map at the edge and handed to Select as an
// argument, exactly like the usage snapshot and the clock. SELECT STAYS PURE.
//
// THE FAIL-OPEN RULE IS THE WHOLE SAFETY ARGUMENT, and it is the same shape as
// the unknown-bucket rule in policy.go. There are three states, not two:
//
//	entry, Available true    the provider answered with models. Usable.
//	entry, Available false   the provider ITSELF answered and serves nothing —
//	                         the CLI is not installed, or is installed and can
//	                         launch no model. Unusable, with the reason quoted.
//	NO ENTRY                 nobody could ask (claudemon down, no bus peer to
//	                         answer claude.listModels, the provider was never
//	                         probed). UNKNOWN, and unknown routes exactly as it
//	                         did before this file existed.
//
// Collapsing the third into the second is the failure this rule exists to
// prevent: a hub that cannot reach claudemon for thirty seconds would otherwise
// declare every provider dead and refuse to route anywhere, which is strictly
// worse than routing to a provider that turns out to be missing — that failure
// is loud, immediate and recoverable, and this one looks like the router
// breaking for no reason.

import (
	"fmt"
	"strings"
)

// ProviderLiveness is one provider's launchability at the moment the map was
// built.
type ProviderLiveness struct {
	// Available is whether work can actually be started on this provider.
	Available bool `json:"available"`
	// Reason is the sentence a refusal quotes — "codex's CLI reports no
	// launchable models", "claudemon could not be reached". Always present on
	// an unavailable entry: a refusal with no reason is one an operator cannot
	// act on.
	Reason string `json:"reason,omitempty"`
	// ObservedAt is when the probe behind this entry answered, in Unix seconds,
	// so a reader can tell a fresh verdict from an old one.
	ObservedAt int64 `json:"observedAt,omitempty"`
}

// ProviderAvailability is provider id -> liveness. A nil map is the ordinary
// state on a hub with no catalog wired at all, and it means "nothing is known",
// never "nothing is available".
type ProviderAvailability map[string]ProviderLiveness

// Unusable answers the walk's question: is there POSITIVE evidence that work
// cannot be started on this provider right now?
//
// A missing entry answers no. That is the fail-open rule, and it is the reason
// this is a method on the map rather than a lookup written out at the call
// site: the moment the check is spelled by hand somewhere, someone writes
// `if !avail[p].Available` and turns "we never asked" into "it is dead".
func (a ProviderAvailability) Unusable(provider string) (string, bool) {
	if len(a) == 0 {
		return "", false
	}
	live, ok := a[normalizeProvider(provider)]
	if !ok || live.Available {
		return "", false
	}
	reason := strings.TrimSpace(live.Reason)
	if reason == "" {
		reason = "it reported no launchable models"
	}
	return fmt.Sprintf("%s is not available to launch right now: %s", normalizeProvider(provider), reason), true
}
