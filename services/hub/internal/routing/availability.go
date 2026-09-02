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
//	entry, Available false   the CLI RAN and reported zero launchable models.
//	                         Unusable, with the reason quoted.
//	NO ENTRY                 nobody could ask, or the ask failed (claudemon
//	                         down, claudemon up and answering non-2xx for this
//	                         provider, no bus peer to answer claude.listModels,
//	                         the provider was never probed). UNKNOWN, and
//	                         unknown routes exactly as it did before this file
//	                         existed.
//
// WHAT THE SECOND STATE DOES NOT CATCH IS A MISSING CLI, and saying otherwise
// has been the standing error in this feature's prose. A binary that is not on
// the machine makes claudemon's list_models spawn fail; claudemon answers 502;
// cmd/hub records answered=false; that is the THIRD state and it fails open.
// The same is true of `claude`, which never sets answered at all (an empty
// alias-and-transcript list is ignorance, not a report), so claude can never
// appear here as unavailable.
//
// Collapsing the third into the second is the failure this rule exists to
// prevent: a hub that cannot reach claudemon for thirty seconds would otherwise
// declare every provider dead and refuse to route anywhere, which is strictly
// worse than routing to a provider that turns out to be missing — that failure
// is loud, immediate and recoverable, and this one looks like the router
// breaking for no reason.
//
// AND IT IS ONLY EVER CONSULTED INSIDE THE FALLOVER WALK (alternatives.go). A
// pinned provider, a capability with no `alternatives:`, and the provider a
// mode shift lands on are none of them checked against this map. That is a
// consequence of where the check lives rather than a rule anybody wrote, and it
// is documented here so nobody reads "unavailable providers are routed around"
// as a property of every answer.

import (
	"fmt"
	"strings"
)

// ProviderLiveness is one provider's launchability at the moment the map was
// built.
type ProviderLiveness struct {
	// Available is whether work can actually be started on this provider.
	Available bool `json:"available"`
	// Reason is the sentence a refusal quotes, and the only thing it may claim
	// is what the probe saw: "codex's CLI ran and reported no launchable
	// model". Always present on an unavailable entry, because a refusal with no
	// reason is one an operator cannot act on. It never says a CLI is missing:
	// a missing CLI does not produce an entry here at all.
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
