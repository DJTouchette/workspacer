package capspec

import (
	"strings"
	"testing"
)

// compositionFloor is the number of pairs on the record today, as a RATCHET.
// A record that shrinks to nothing satisfies every loop below and asserts
// nothing — the failure mode this whole family of guards keeps re-learning.
const compositionFloor = 11

// TestCompositionRecordIsWellFormed holds the record to the shape that makes it
// checkable at all. A pair whose halves are not real capabilities, or whose
// status is neither "closed" nor "accepted", is prose in a Go file.
func TestCompositionRecordIsWellFormed(t *testing.T) {
	all := Compositions()
	if len(all) < compositionFloor {
		t.Fatalf("the composition record holds %d pairs (floor %d) — it shrank, and an empty record passes every check in this file", len(all), compositionFloor)
	}
	shapes := map[CompositionShape]bool{ShapeWriteThenInterpret: true, ShapeWidenThenUse: true}
	seenShape := map[CompositionShape]int{}
	names := map[string]bool{}
	for _, c := range all {
		if names[c.Name] {
			t.Errorf("two compositions share the name %q — a failure would name the wrong one", c.Name)
		}
		names[c.Name] = true
		if !shapes[c.Shape] {
			t.Errorf("%q has shape %q, which is not one of the two generalised forms", c.Name, c.Shape)
		}
		seenShape[c.Shape]++
		if len(strings.TrimSpace(c.Crossing)) < 80 {
			t.Errorf("%q has no real crossing statement (%q) — the crossing is the whole content of a composition record: what each half is allowed to do, and what the two are together", c.Name, c.Crossing)
		}
		for _, half := range []string{c.A, c.B} {
			if _, guarded := EventTopicCapability(half); guarded {
				continue // an event topic is a legitimate half
			}
			if MissingClassification(half) {
				t.Errorf("%q names %q as a half, and that is neither a classified capability nor a guarded event topic — a composition between things nobody classified cannot be reasoned about", c.Name, half)
			}
		}
		if c.A == c.B {
			t.Errorf("%q composes %q with itself", c.Name, c.A)
		}
		closed := strings.TrimSpace(c.ClosedBy) != ""
		accepted := len(c.AcceptedIn) > 0
		switch {
		case closed && accepted:
			t.Errorf("%q is both ClosedBy %q and AcceptedIn %v — a pair is either no stronger than its halves or deliberately granted together, and a reader cannot act on both", c.Name, c.ClosedBy, c.AcceptedIn)
		case !closed && !accepted:
			t.Errorf("%q is neither closed nor accepted. That is the state every one of these pairs shipped in: written down nowhere, so 'decided and safe' was indistinguishable from 'nobody looked'.", c.Name)
		}
		for _, tier := range c.AcceptedIn {
			if tier == "operator" {
				t.Errorf("%q lists the operator tier, which holds every capability by definition — listing it makes the tier guard vacuous for this pair", c.Name)
			}
		}
	}
	// BOTH shapes, or the record documents one species of defect and the guard
	// that reads it only ever exercises that species.
	for shape := range shapes {
		if seenShape[shape] == 0 {
			t.Errorf("no composition on the record has shape %q — the record covers one of the two generalised forms and is silent about the other", shape)
		}
	}
}

// TestClosedCompositionsNameALiveMechanism is the anti-prose check: a ClosedBy
// sentence has to point at something that exists. Each named mechanism is a
// symbol in this repo, and this asserts the name is at least spelled the way the
// code spells it — cheap, and it catches the rename that turns a closure record
// into folklore.
func TestClosedCompositionsNameALiveMechanism(t *testing.T) {
	// symbol → the file that must contain it.
	mechanisms := map[string][]string{
		"pathIsAgentInterpretedConfig": {"services", "hub", "cmd", "brain", "fsguard.go"},
		"isAgentInterpretedConfigPath": {"apps", "desktop", "src", "main", "lib", "pathConfinement.ts"},
		"resultPathIsSecret":           {"services", "hub", "cmd", "brain", "search.go"},
		"isSecretResultPath":           {"apps", "desktop", "src", "main", "lib", "pathConfinement.ts"},
		"scrubAdoptedSpawnFields":      {"services", "hub", "internal", "layout", "layout.go"},
		"isVolumeRoot":                 {"services", "hub", "internal", "plugin", "manager.go"},
		"guardReplaySession":           {"apps", "desktop", "src", "main", "services", "hubCapabilities.ts"},
		"EventTopicSpec":               {"services", "hub", "internal", "bus", "bus.go"},
		"SubscribeFiltered":            {"services", "hub", "internal", "bus", "bus.go"},
		"scrubBootDocumentAgents":      {"services", "hub", "cmd", "brain", "bootdoc.go"},
		"validatePushEndpoint":         {"services", "hub", "internal", "push", "endpoint.go"},
	}
	checked := 0
	for symbol, file := range mechanisms {
		data := mustReadRepoFile(t, file...)
		if !strings.Contains(string(data), symbol) {
			t.Errorf("a ClosedBy record names %q, and %s does not contain it — the mechanism was renamed or removed and the closure record became folklore", symbol, strings.Join(file, "/"))
		}
		checked++
	}
	if checked != len(mechanisms) {
		t.Fatalf("checked %d of %d mechanisms", checked, len(mechanisms))
	}
	// And every ClosedBy sentence must cite at least one of them, so a new pair
	// cannot be closed by a sentence that names nothing.
	for _, c := range Compositions() {
		if strings.TrimSpace(c.ClosedBy) == "" {
			continue
		}
		cited := false
		for symbol := range mechanisms {
			if strings.Contains(c.ClosedBy, symbol) {
				cited = true
				break
			}
		}
		if !cited {
			t.Errorf("%q is marked closed but its ClosedBy names none of the mechanisms this test can verify — add the symbol to the map above, or the closure is unfalsifiable", c.Name)
		}
	}
}

// THE FORCING FUNCTION, and the answer to "can the record be derived".
//
// It cannot — the measurements are in composition.go's header (0.95% precision,
// hits carried by English filler, 6% of `executes` cells naming a joinable
// object, and the deciding fact of every recorded chain living in a third-party
// interpreter outside this repo). So the record is forced instead: every
// capability that carries a caller value the host acts on must be a recorded
// half or carry a WRITTEN reason it cannot be one.
//
// This is the guard OPEN 2 asked for. Before it, injecting a new capability
// whose own excuse described a write-then-interpret half left every composition
// check green: `go test ./internal/capspec -run Composition` printed two PASSes
// for a method nobody had considered.
func TestEveryActingCapabilityHasBeenConsideredForComposition(t *testing.T) {
	unconsidered := CompositionUnconsidered()
	if len(unconsidered) > 0 {
		t.Errorf(`%d capabilities carry a caller value the host acts on and have neither been recorded as a composition half nor written down as inert:

    %s

Answer both shapes for each, in compositionInert (composition.go):

    WRITE-THEN-INTERPRET  do the bytes this call writes end up somewhere another
                          capability, the host, or a THIRD-PARTY interpreter
                          reads as CONFIG, CODE, ARGV or POLICY? (Three of the
                          recorded pairs turn on a file the interpreter reads
                          that nothing in this repo mentions.)
    WIDEN-THEN-USE        does this call change state — a grant, a root set, a
                          permission mode, an approval gate, a session — that
                          some other guard CONSULTS?

"It is safe" is not an answer: every half of every recorded pair is safe.`,
			len(unconsidered), strings.Join(unconsidered, "\n    "))
	}
}

// The record has to be a record. An entry that is neither a recorded half nor a
// real sentence is silence with a key next to it, which is the state this whole
// family of guards keeps re-learning to reject.
func TestCompositionInertReasonsAreReasons(t *testing.T) {
	halves := map[string]bool{}
	for _, h := range CompositionHalves() {
		halves[h] = true
	}
	actors := map[string]bool{}
	for _, m := range compositionActors() {
		actors[m] = true
	}
	written, blank := 0, 0
	for method, reason := range compositionInert {
		if !actors[method] {
			t.Errorf("compositionInert names %q, which is not a capability that carries a caller value the host acts on — an entry for a method the forcing function never asks about is dead weight that makes the record look more complete than it is", method)
			continue
		}
		if strings.TrimSpace(reason) == "" {
			// The empty form is reserved for methods that ARE recorded halves;
			// anything else is an unwritten reason.
			if !halves[method] {
				t.Errorf("%q has an EMPTY inert reason and is not a recorded composition half — the blank form means 'see the pair above', and there is no pair", method)
			}
			blank++
			continue
		}
		if halves[method] {
			t.Errorf("%q is a recorded composition half AND carries an inert reason (%q) — a reader cannot act on both", method, reason)
		}
		if len(reason) < 60 {
			t.Errorf("%q's inert reason is too short to be one: %q. It has to say what this call writes or changes and why nothing downstream reads it as instruction.", method, reason)
		}
		written++
	}
	if written < 30 {
		t.Fatalf("only %d written inert reasons — the record shrank, and a shrunken record satisfies the forcing function above by covering fewer methods", written)
	}
	if blank == 0 {
		t.Fatal("no recorded half is cross-referenced in compositionInert — the two halves of the record have stopped pointing at each other")
	}
}

// The population itself must not quietly empty. A forcing function over an empty
// set is a PASS that asserts nothing — the exact failure every scan-based guard
// in this repo has had to be taught.
func TestCompositionActorPopulationIsReal(t *testing.T) {
	actors := compositionActors()
	if len(actors) < 50 {
		t.Fatalf("the forcing function covers only %d capabilities (%v) — PathParam or unscopedByDecision stopped being enumerable and the guard is guarding nothing", len(actors), actors)
	}
	// Both sources must contribute, or one of them is being skipped.
	fromPath, fromUnscoped := 0, 0
	for _, m := range actors {
		if _, ok := PathParam[m]; ok {
			fromPath++
			continue
		}
		fromUnscoped++
	}
	if fromPath < 5 || fromUnscoped < 20 {
		t.Fatalf("population = %d path-scoped + %d unscoped-by-decision; one source has collapsed", fromPath, fromUnscoped)
	}
}
