package authtoken

import (
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// scopedTiers are the credential tiers that are an ALLOWLIST. Operator is
// excluded on purpose: it returns "*" and holds every composition by
// definition, so including it would make every check here vacuous.
var scopedTiers = map[string]Scope{"view": ScopeView, "triage": ScopeTriage}

// TestScopedTiersDoNotSilentlyAcquireACompositionIsTheGuardThisRoundAdded holds
// each scoped tier to capspec's composition record.
//
// The record exists because every other guard in this repo asks whether ONE call
// can escape. A pair whose halves are each correctly authorized is invisible to
// all of them, and the triage tier is the proof: agents.sendMessage's own capspec
// excuse is that "the agent's own tool approvals are the gate", and triage also
// holds claude.approve — the RESOLVER of exactly those approvals, as claude.approve's
// own entry says. A triage token can therefore tell an agent that may run a shell
// to run one, and then approve it, without holding terminals.create,
// sessions.terminalInput, fs.write, git.* or agents.spawn — every one of which
// the tier's doc comment lists as "deliberately absent" and every one of which is
// genuinely refused per call.
//
// That pair is a real product decision (the phone's whole job is replying to an
// agent and answering its prompts), so it is on the record as AcceptedIn:
// ["triage"] rather than pretended away. What this test buys is everything that
// comes next: a tier that acquires the OTHER half of a pair it was not accepted
// for fails here, at the line that added the method.
func TestScopedTiersDoNotSilentlyAcquireAComposition(t *testing.T) {
	comps := capspec.Compositions()
	if len(comps) == 0 {
		t.Fatal("capspec's composition record is empty — this guard reads it, so an empty record makes it a no-op")
	}

	tierNames := make([]string, 0, len(scopedTiers))
	for name := range scopedTiers {
		tierNames = append(tierNames, name)
	}
	sort.Strings(tierNames)

	checkedPairs, closedPairsHeld := 0, 0
	for _, tierName := range tierNames {
		held := map[string]bool{}
		for _, m := range scopedTiers[tierName].Methods() {
			held[m] = true
		}
		if len(held) == 0 {
			t.Fatalf("tier %q grants no methods — Scope.Methods changed and this guard is comparing against nothing", tierName)
		}
		for _, c := range comps {
			if !held[c.A] || !held[c.B] {
				continue
			}
			// A pair marked ClosedBy is, by the record's own definition, "no
			// stronger than its halves" — the named mechanism makes the
			// composition add nothing, and TestClosedCompositionsNameALiveMechanism
			// holds that mechanism to being a symbol that still exists. Holding
			// both halves of such a pair is therefore not an acquisition, and
			// demanding an AcceptedIn for it would force the record to say two
			// contradictory things about the same pair (the well-formedness test
			// forbids being both closed and accepted, for exactly the reason that
			// a reader cannot act on both).
			//
			// This is the only relaxation in this file, and it is bounded by the
			// floor below: pairs that are OPEN — accepted, or neither closed nor
			// accepted — are still every one of them evaluated, and at least one
			// must be.
			if strings.TrimSpace(c.ClosedBy) != "" {
				closedPairsHeld++
				continue
			}
			checkedPairs++
			accepted := false
			for _, at := range c.AcceptedIn {
				if at == tierName {
					accepted = true
				}
			}
			if !accepted {
				t.Errorf(`the %q tier holds BOTH halves of a composition it was never accepted for.

  pair:     %s + %s
  what:     %s
  crossing: %s

Each half is authorized on its own and no per-call guard can see the pair. Either
drop one half from the tier, or add %q to that composition's AcceptedIn with the
reason it is a deliberate product decision — but it has to be a decision somebody
made, not a method that arrived next to another one.`,
					tierName, c.A, c.B, c.Name, c.Crossing, tierName)
			}
		}
	}
	// THE FLOOR. Every check above is `continue` when a tier holds neither half,
	// so a record of pairs nobody's tier holds — or a tier list that emptied —
	// passes in silence. At least one pair must actually have been evaluated,
	// and today exactly one is: agents.sendMessage + claude.approve in triage.
	if checkedPairs == 0 {
		t.Fatal("no scoped tier holds both halves of any recorded composition, so this guard evaluated nothing. Either the tiers narrowed (delete this floor and say so) or the record stopped naming methods the tiers grant.")
	}
	// And the relaxation must not become the whole story: if EVERY held pair is
	// a closed one, this guard has skipped its way to a pass.
	if closedPairsHeld > 0 && checkedPairs == 0 {
		t.Fatalf("every composition a scoped tier holds (%d) was skipped as closed", closedPairsHeld)
	}
}

// TestAcceptedCompositionsAreStillHeld is the mirror image, and it is what keeps
// the record from rotting into a list of pairs nobody has any more. An
// AcceptedIn entry is a claim about a tier; if the tier no longer holds both
// halves, the acceptance is stale and the next reader will believe a bound that
// is stronger than reality — the same defect as an excuse naming a gate nobody
// arms.
func TestAcceptedCompositionsAreStillHeld(t *testing.T) {
	for _, c := range capspec.Compositions() {
		for _, tierName := range c.AcceptedIn {
			scope, ok := scopedTiers[tierName]
			if !ok {
				t.Errorf("%q is accepted in tier %q, which is not a scoped tier this package defines (%v)", c.Name, tierName, scopedTierNames())
				continue
			}
			held := map[string]bool{}
			for _, m := range scope.Methods() {
				held[m] = true
			}
			if !held[c.A] || !held[c.B] {
				t.Errorf("%q is recorded as accepted in the %q tier, but that tier no longer holds both halves (%s: %v, %s: %v). Remove the acceptance — a stale one describes a bound the tier does not have.",
					c.Name, tierName, c.A, held[c.A], c.B, held[c.B])
			}
		}
	}
}

// TestTriageDeliberateAbsencesAreStillAbsent pins the OTHER half of the triage
// tier's own doc comment, which reads: "Deliberately absent: agents.spawn (the
// /m spawn tab is operator surface), terminals.*, git.*, fs.*, config.save,
// plugin/config admin."
//
// Every one of those denials is real and enforced per call, and the
// sendMessage+approve pair walks around all of them — the tier cannot run a
// shell, write a file, commit or spawn, but it can tell an agent that already can
// to do so and then approve it. Both facts have to stay true together for the
// record above to be an honest description: this is the "still absent" half.
func TestTriageDeliberateAbsencesAreStillAbsent(t *testing.T) {
	held := map[string]bool{}
	for _, m := range ScopeTriage.Methods() {
		held[m] = true
	}
	for _, absent := range []string{
		"agents.spawn",
		"terminals.create",
		"sessions.terminalInput",
		"sessions.attachTerminal",
		"fs.write",
		"fs.read",
		"git.commit",
		"git.push",
		"config.save",
		"layout.set",
		// claude.gate is absent too, and its absence is the thing the ORIGINAL
		// theory of this chain got wrong: gate only ADDS parking, so the pair
		// never needed it. Pinned so a future "the phone should be able to arm
		// the gate" lands here rather than quietly making the tier stronger.
		"claude.gate",
	} {
		if held[absent] {
			t.Errorf("the triage tier now grants %q, which its own doc comment lists as deliberately absent. Composed with agents.sendMessage + claude.approve — which it already holds — that changes what a phone token is.", absent)
		}
	}
	// The floor: the tier must still hold the two halves the record is about, or
	// the absences above are being asserted of a tier that no longer exists.
	for _, must := range []string{"agents.sendMessage", "claude.approve"} {
		if !held[must] {
			t.Fatalf("the triage tier no longer grants %q — update capspec's composition record, which claims it does", must)
		}
	}
}

func scopedTierNames() string {
	names := make([]string, 0, len(scopedTiers))
	for n := range scopedTiers {
		names = append(names, n)
	}
	sort.Strings(names)
	return strings.Join(names, ", ")
}
