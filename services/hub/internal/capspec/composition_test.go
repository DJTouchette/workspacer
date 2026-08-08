package capspec

import (
	"strings"
	"testing"
)

// compositionFloor is the number of pairs on the record today, as a RATCHET.
// A record that shrinks to nothing satisfies every loop below and asserts
// nothing — the failure mode this whole family of guards keeps re-learning.
const compositionFloor = 8

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
		"EventTopicCapability":         {"services", "hub", "internal", "bus", "bus.go"},
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
