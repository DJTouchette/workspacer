package capspec

import (
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
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
		// A closure with no Bearing is the state this round found: a sentence
		// citing a live symbol, checked for nothing else, and honoured by the
		// tier guard as an exemption.
		if closed && len(c.Bearings) == 0 {
			t.Errorf("%q is marked ClosedBy %q and carries NO Bearing. An exemption anybody can mint by naming a symbol is not an exemption: internal/authtoken's tier guard SKIPS closed pairs, so this one silences itself.", c.Name, c.ClosedBy)
		}
		if !closed && len(c.Bearings) > 0 {
			t.Errorf("%q is not closed but carries %d Bearing(s) — a proof of closure attached to a pair that is accepted, not closed, is a reader trap", c.Name, len(c.Bearings))
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

// closureMechanisms is symbol → the file that must contain it.
var closureMechanisms = map[string][]string{
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

// TestClosedCompositionsNameALiveMechanism is the anti-prose check: a ClosedBy
// sentence has to point at something that exists. Each named mechanism is a
// symbol in this repo, and this asserts the name is at least spelled the way the
// code spells it — cheap, and it catches the rename that turns a closure record
// into folklore.
//
// It is NOT sufficient on its own, and that was this round's finding: "a symbol
// that exists somewhere in the tree" is satisfied by ANY symbol, including one
// from an unrelated package. See
// TestClosedCompositionsProveTheirGuardReachesTheirHalves for the half that
// makes the citation mean something.
func TestClosedCompositionsNameALiveMechanism(t *testing.T) {
	checked := 0
	for symbol, file := range closureMechanisms {
		data := mustReadRepoFile(t, file...)
		if !strings.Contains(string(data), symbol) {
			t.Errorf("a ClosedBy record names %q, and %s does not contain it — the mechanism was renamed or removed and the closure record became folklore", symbol, strings.Join(file, "/"))
		}
		checked++
	}
	if checked != len(closureMechanisms) {
		t.Fatalf("checked %d of %d mechanisms", checked, len(closureMechanisms))
	}
	// And every ClosedBy sentence must cite at least one of them, so a new pair
	// cannot be closed by a sentence that names nothing.
	for _, c := range Compositions() {
		if strings.TrimSpace(c.ClosedBy) == "" {
			continue
		}
		cited := false
		for symbol := range closureMechanisms {
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

// ── the proof engine ────────────────────────────────────────────────────────

// repoText reads a repo file once per test.
func repoText(t *testing.T, cache map[string]string, file []string) string {
	t.Helper()
	key := filepath.Join(file...)
	if s, ok := cache[key]; ok {
		return s
	}
	s := string(mustReadRepoFile(t, file...))
	cache[key] = s
	return s
}

// goFuncBody returns the source of a top-level Go function or method, using the
// property gofmt guarantees: the closing brace of a top-level declaration is the
// only `}` at column 0 after its header.
func goFuncBody(text, symbol string) (string, bool) {
	head := regexp.MustCompile(`(?m)^func (?:\([^)]*\) )?` + regexp.QuoteMeta(symbol) + `\(`)
	loc := head.FindStringIndex(text)
	if loc == nil {
		return "", false
	}
	tail := text[loc[1]:]
	end := regexp.MustCompile(`(?m)^\}`).FindStringIndex(tail)
	if end == nil {
		return "", false
	}
	return text[loc[0] : loc[1]+end[1]], true
}

func callsSymbol(body, symbol string) bool {
	return regexp.MustCompile(`\b` + regexp.QuoteMeta(symbol) + `\(`).MatchString(body)
}

// verifyBearing is the whole point of this round: it decides whether a named
// guard actually REACHES a named capability, in source, rather than merely
// existing. `partners` is the set of capabilities this bearing is allowed to be
// about — a composition's two halves, or the one method an inert claim covers.
// otherActorsNamedIn reports which composition actors OTHER than `on` appear as
// string literals in a function body. A non-empty result means the body routes
// rather than answers, so naming `on` there is a coincidence of dispatch.
//
// It reads capspec's own actor list rather than a hand-kept roster of dispatchers,
// so a new dispatch switch is caught the day it is written and a shrinking actor
// list cannot quietly re-open the door (TestCompositionRecordNamesOnlyRegistered-
// Capabilities holds that list to the registrations).
func otherActorsNamedIn(body, on string) []string {
	var out []string
	for _, m := range CompositionActorsForTest() {
		if m == on {
			continue
		}
		if regexp.MustCompile(`["'` + "`" + `]` + regexp.QuoteMeta(m) + `\b`).MatchString(body) {
			out = append(out, m)
		}
	}
	sort.Strings(out)
	return out
}

func verifyBearing(t *testing.T, cache map[string]string, where string, b Bearing, partners []string) {
	t.Helper()
	if strings.TrimSpace(b.Symbol) == "" || strings.TrimSpace(b.On) == "" {
		t.Errorf("%s: a bearing with no On/Symbol proves nothing", where)
		return
	}
	held := false
	for _, p := range partners {
		if p == b.On {
			held = true
		}
	}
	if !held {
		t.Errorf("%s: the bearing is On %q, which is not one of %v — a guard that acts on somebody ELSE's capability says nothing about this one", where, b.On, partners)
		return
	}

	switch b.Kind {
	case BearsAtCallSite:
		text := repoText(t, cache, b.Entry.File)
		entryFile := filepath.Join(b.Entry.File...)
		if b.ByArg {
			call := regexp.MustCompile(`\b` + regexp.QuoteMeta(b.Entry.Symbol) + `\(\s*['"]` + regexp.QuoteMeta(b.On) + `['"]`)
			if !call.MatchString(text) {
				t.Errorf("%s: claims %s reaches %s, but %s contains no call %s('%s', …). The guard has to be invoked with the capability's OWN name or it belongs to whichever registration it happens to sit next to.",
					where, b.Symbol, b.On, entryFile, b.Entry.Symbol, b.On)
				return
			}
		} else {
			body, ok := goFuncBody(text, b.Entry.Symbol)
			if !ok {
				t.Errorf("%s: the chain is entered at %s in %s, and no such Go function is defined there", where, b.Entry.Symbol, entryFile)
				return
			}
			// Named inside a string literal in that body: the handler's own
			// error message ("layout.set requires { data }") is how a Go
			// provider spells which method it is answering.
			named := regexp.MustCompile(`["'` + "`" + `]` + regexp.QuoteMeta(b.On) + `\b`)
			if !named.MatchString(body) {
				t.Errorf("%s: the chain is entered at %s in %s, but that function's body never names %q — nothing ties this guard to this capability", where, b.Entry.Symbol, entryFile, b.On)
				return
			}
			// …but naming it is only evidence if the site is ABOUT it. This form
			// was a skeleton key: cmd/brain/handlers.go's `handle` is a 192-line
			// dispatch switch naming 50 of capspec's methods, so a bearing entered
			// there "proved" a guard reached almost any capability — which re-mints
			// the fabricated exemption the Bearing type exists to refuse, and a
			// WitnessGuarded for claude.signal, whose own reason says no guard
			// names it.
			//
			// A dispatcher names every method it routes; a handler names the one it
			// answers. Counting is what tells them apart, and it needs no
			// allow-list of "known dispatchers" that would itself rot.
			if others := otherActorsNamedIn(body, b.On); len(others) > 0 {
				t.Errorf("%s: the chain is entered at %s in %s, but that function also names %d other capabilities (%v…) — it is a dispatcher, not a proof about %q. Enter the chain at the handler that answers %q alone, or use the ByArg form where the guard is called with the capability's own name.",
					where, b.Entry.Symbol, entryFile, len(others), others[:min(3, len(others))], b.On, b.On)
				return
			}
		}
		if len(b.Chain) == 0 {
			if b.Entry.Symbol != b.Symbol {
				t.Errorf("%s: entry %s is not the named guard %s and no chain connects them", where, b.Entry.Symbol, b.Symbol)
			}
			return
		}
		if b.Chain[0].Symbol != b.Entry.Symbol {
			t.Errorf("%s: the chain starts at %s but the entry call is %s — the proof does not begin where the capability is named", where, b.Chain[0].Symbol, b.Entry.Symbol)
			return
		}
		prevBody := ""
		for i, site := range b.Chain {
			body, ok := goFuncBody(repoText(t, cache, site.File), site.Symbol)
			if !ok {
				t.Errorf("%s: chain link %d names %s in %s, and no such Go function is defined there — the path from %s to %s is broken",
					where, i, site.Symbol, filepath.Join(site.File...), b.On, b.Symbol)
				return
			}
			if i > 0 && !callsSymbol(prevBody, site.Symbol) {
				t.Errorf("%s: %s does not call %s, so the claimed path from %s to %s does not exist. A closure record is only worth the call chain behind it.",
					where, b.Chain[i-1].Symbol, site.Symbol, b.On, b.Symbol)
				return
			}
			prevBody = body
		}
		if last := b.Chain[len(b.Chain)-1].Symbol; last != b.Symbol {
			t.Errorf("%s: the chain ends at %s, not at the named guard %s", where, last, b.Symbol)
		}

	case BearsInTopicRegistry:
		method, guardedTopic := EventTopicCapability(b.On)
		if !guardedTopic {
			t.Errorf("%s: claims the topic registry binds %q, but EventTopicCapability says it is not guarded by any capability — the registry stopped being the mechanism this pair is closed by", where, b.On)
			return
		}
		bound := false
		for _, p := range partners {
			if p == method && p != b.On {
				bound = true
			}
		}
		if !bound {
			t.Errorf("%s: the registry guards %q with %q, which is not the other half of this pair (%v). The closure claims the two planes now answer the same question; the registry says they do not.", where, b.On, method, partners)
		}

	case BearsOnGrantedRoots:
		// The weakest kind, and its limit is enforced rather than described: it
		// is admissible only for a half the bus actually confines by root set.
		if _, scoped := IsPathScoped(b.On); !scoped {
			t.Errorf("%s: %s is claimed to close this pair by narrowing the granted roots, but %q is not path-scoped, so no root set governs it", where, b.Symbol, b.On)
			return
		}
		body, ok := goFuncBody(repoText(t, cache, b.Entry.File), b.Entry.Symbol)
		if !ok {
			t.Errorf("%s: %s is not defined in %s", where, b.Entry.Symbol, filepath.Join(b.Entry.File...))
			return
		}
		if !callsSymbol(body, b.Symbol) {
			t.Errorf("%s: %s does not call %s — the narrowing this pair is closed by is not applied where the roots are built", where, b.Entry.Symbol, b.Symbol)
			return
		}
		// …and it must be narrowing the roots the BUS stores, not some local
		// string: the same function has to go through the bus's own root
		// canonicalization, which is what a granted fsRoot is.
		if !callsSymbol(body, "bus.CanonicalizeRoot") && !callsSymbol(body, "bus.PathIsSecret") {
			t.Errorf("%s: %s narrows something, but not a root the bus registers (it calls neither bus.CanonicalizeRoot nor bus.PathIsSecret) — a granted-roots closure has to act on granted roots", where, b.Entry.Symbol)
		}

	default:
		t.Errorf("%s: bearing kind %q is not one of the three proofs this test knows how to run", where, b.Kind)
	}
}

// TestClosedCompositionsProveTheirGuardReachesTheirHalves is the guard this
// round exists for.
//
// Before it, ClosedBy was checked for exactly one thing: that it cited a symbol
// which occurs in some file. Two mutations survived the entire Go suite:
//
//  1. replacing a real closure's reason with an unrelated live symbol from
//     another package;
//  2. adding a NEW pair whose halves the `triage` tier genuinely holds
//     (sessions.snapshot + agents.sendMessage), marking it ClosedBy with a
//     symbol that has nothing to do with either — which SILENCES
//     internal/authtoken's tier guard, because that guard skips closed pairs on
//     the record's own promise that a closed pair adds nothing.
//
// So a closure now has to show its work: the guard is entered at a site naming
// one of THIS pair's halves, and every hop from there to the guard is a real
// call in the previous hop's own body. The pattern is
// TestUnscopedByDecisionProviderClaimsAreTrue's — an excuse held to its own word.
func TestClosedCompositionsProveTheirGuardReachesTheirHalves(t *testing.T) {
	cache := map[string]string{}
	proven := 0
	for _, c := range Compositions() {
		if strings.TrimSpace(c.ClosedBy) == "" {
			continue
		}
		if len(c.Bearings) == 0 {
			t.Errorf("%q is closed and proves nothing", c.Name)
			continue
		}
		for i, b := range c.Bearings {
			where := c.Name + " (bearing " + strconv.Itoa(i) + ")"
			// The prose and the proof must name the same guard, or the sentence
			// a human reads and the symbol a machine checks drift apart — which
			// is exactly how the record got here.
			if !strings.Contains(c.ClosedBy, b.Symbol) {
				t.Errorf("%s: the bearing proves %q, which the ClosedBy sentence never names (%q)", where, b.Symbol, c.ClosedBy)
			}
			if _, known := closureMechanisms[b.Symbol]; !known {
				t.Errorf("%s: %q is not in closureMechanisms, so nothing checks it still exists under that name", where, b.Symbol)
			}
			verifyBearing(t, cache, where, b, []string{c.A, c.B})
			proven++
		}
	}
	// The floor. Every branch above is a `continue` on an unclosed pair, so a
	// record with no closures at all would pass in silence.
	if proven < 10 {
		t.Fatalf("only %d bearings were verified — the closures stopped carrying proofs and this guard is guarding nothing", proven)
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
func TestEveryActingCapabilityHasBeenConsideredForComposition(t *testing.T) {
	unconsidered := CompositionUnconsidered()
	if len(unconsidered) > 0 {
		t.Errorf(`%d capabilities carry a caller value the host acts on and have neither been recorded as a composition half nor written down as inert:

    %s

Answer both shapes for each, in compositionInert (composition.go), WITH A WITNESS:

    WRITE-THEN-INTERPRET  do the bytes this call writes end up somewhere another
                          capability, the host, or a THIRD-PARTY interpreter
                          reads as CONFIG, CODE, ARGV or POLICY? (Three of the
                          recorded pairs turn on a file the interpreter reads
                          that nothing in this repo mentions.)
    WIDEN-THEN-USE        does this call change state — a grant, a root set, a
                          permission mode, an approval gate, a session — that
                          some other guard CONSULTS?

"It is safe" is not an answer: every half of every recorded pair is safe. Neither
is a paragraph on its own — see TestInertClaimsCarryACheckedWitness.`,
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
	for method, claim := range compositionInert {
		if !actors[method] {
			t.Errorf("compositionInert names %q, which is not a capability that carries a caller value the host acts on — an entry for a method the forcing function never asks about is dead weight that makes the record look more complete than it is", method)
			continue
		}
		if strings.TrimSpace(claim.Reason) == "" {
			// The empty form is reserved for methods that ARE recorded halves;
			// anything else is an unwritten reason.
			if !halves[method] {
				t.Errorf("%q has an EMPTY inert reason and is not a recorded composition half — the blank form means 'see the pair above', and there is no pair", method)
			}
			if len(claim.Witnesses) > 0 {
				t.Errorf("%q is a recorded half and also carries witnesses — the pair is the evidence", method)
			}
			blank++
			continue
		}
		if halves[method] {
			t.Errorf("%q is a recorded composition half AND carries an inert reason (%q) — a reader cannot act on both", method, claim.Reason)
		}
		if len(claim.Reason) < 60 {
			t.Errorf("%q's inert reason is too short to be one: %q. It has to say what this call writes or changes and why nothing downstream reads it as instruction.", method, claim.Reason)
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

// unwitnessedInertClaims is the CLOSED set of inert claims that rest on prose
// alone: nothing in the repo can be read to check them, and saying so is the
// honest form. Two are about what a HUMAN does with a file of text; one is about
// a signal that carries no classified value at all.
//
// This list is the point. Before it, a false inert reason cost one paragraph —
// injecting a method into unscopedByDecision and pairing it with 200 plausible
// characters here satisfied the forcing function exactly as well as the truth
// did. Adding a member now fails this test BY NAME, so "no evidence" is a
// decision somebody has to make in the open.
var unwitnessedInertClaims = map[string]bool{
	"claude.signal":            true,
	"claude.handoffBrief":      true,
	"claude.handoffAgentBrief": true,
}

// TestInertClaimsCarryACheckedWitness makes the other half of the record
// checkable.
//
// An inert claim's sentence cannot be verified — nothing proves the absence of
// an interpreter. What CAN be verified is the load-bearing fact each sentence
// rests on, and every claim now names one: the confinement call that carries its
// own method name; the widening twin it is the undo of; the topic registry entry
// that makes it a gate; the per-parameter decisions capspec already holds. The
// sentence must name what the witness checks, so prose and evidence cannot drift.
func TestInertClaimsCarryACheckedWitness(t *testing.T) {
	cache := map[string]string{}
	halves := map[string]bool{}
	for _, h := range CompositionHalves() {
		halves[h] = true
	}
	byKind := map[WitnessKind]int{}
	sawUnwitnessed := map[string]bool{}

	for _, method := range sortedFixtureMethods(compositionInert) {
		claim := compositionInert[method]
		if strings.TrimSpace(claim.Reason) == "" {
			continue // recorded half; covered by the test above
		}
		if len(claim.Witnesses) == 0 {
			t.Errorf("%q is written down as inert with no witness at all — not even the explicit 'nothing here is checkable' marker. A paragraph is what the forcing function used to accept, and a false paragraph passed it.", method)
			continue
		}
		for _, w := range claim.Witnesses {
			byKind[w.Kind]++
			switch w.Kind {
			case WitnessGuarded:
				if w.Bearing == nil {
					t.Errorf("%q claims a guard witness with no bearing", method)
					continue
				}
				verifyBearing(t, cache, method, *w.Bearing, []string{method})
				if !strings.Contains(claim.Reason, w.Bearing.Symbol) {
					t.Errorf("%q rests on %s, and its own sentence never names it: %q", method, w.Bearing.Symbol, claim.Reason)
				}

			case WitnessNarrows:
				verifyNarrows(t, method, claim, w.Widens, halves)

			case WitnessTopicGuard:
				gate, guardedTopic := EventTopicCapability(w.Topic)
				if !guardedTopic || gate != method {
					t.Errorf("%q claims the event registry makes it the gate on %q, but the registry says %q (guarded=%v) — the claim that 'its output is governed where the output lives' is false",
						method, w.Topic, gate, guardedTopic)
				}
				if !strings.Contains(claim.Reason, w.Topic) {
					t.Errorf("%q rests on the %s topic and never names it", method, w.Topic)
				}

			case WitnessParamsClassified:
				if len(w.Params) == 0 {
					t.Errorf("%q claims a per-parameter witness naming no parameter", method)
				}
				for _, p := range w.Params {
					status, decision := ClassifyParam(method, p)
					if status == ParamUnclassified {
						t.Errorf("%q rests on %q being classified per-parameter, and capspec classifies no such param on this method — the sentence describes a record that does not exist", method, p)
						continue
					}
					if decision.Kind == KindInert {
						t.Errorf("%q rests on %q, whose decision is KindInert: an inert param is capspec saying the value becomes nothing, so it cannot be the evidence that a dangerous value was considered", method, p)
					}
					if !reasonNamesParam(claim.Reason, p) {
						t.Errorf("%q rests on the decision recorded for %q, and its own sentence never names that parameter: %q", method, p, claim.Reason)
					}
				}

			case WitnessNone:
				sawUnwitnessed[method] = true
				if !unwitnessedInertClaims[method] {
					t.Errorf(`%q claims the WitnessNone form — "nothing about this is machine-checkable".

That is a CLOSED set (unwitnessedInertClaims, composition_test.go) with three
members, and this method is not one of them. A new prose-only claim is how a
FALSE inert reason gets in: it costs one paragraph and satisfies the forcing
function exactly as well as a true one. Either give it a witness — a confinement
call naming it, a widening twin it undoes, a topic it gates, a classified param —
or add it here deliberately and say why nothing can check it.`, method)
				}
				if !strings.Contains(claim.Reason, "NOTHING HERE IS MACHINE-CHECKED") {
					t.Errorf("%q rests on prose alone and does not say so in its own reason (expected the marker NOTHING HERE IS MACHINE-CHECKED): %q", method, claim.Reason)
				}

			default:
				t.Errorf("%q carries witness kind %q, which is not in the vocabulary", method, w.Kind)
			}
		}
	}

	// The pinned set must still be exactly what it says it is: an entry that
	// quietly acquired a witness (or vanished) leaves a stale exemption behind,
	// and a stale exemption is a hole somebody can grow into.
	for method := range unwitnessedInertClaims {
		if !sawUnwitnessed[method] {
			t.Errorf("unwitnessedInertClaims pins %q as prose-only, and it no longer claims that form — remove it from the pin, or the exemption is available to whatever moves in next", method)
		}
	}
	// Floors: the vocabulary must actually be exercised, or a record that
	// collapsed onto one trivial witness kind would pass.
	for _, kind := range []WitnessKind{WitnessGuarded, WitnessNarrows, WitnessParamsClassified, WitnessTopicGuard} {
		if byKind[kind] == 0 {
			t.Errorf("no inert claim uses the %q witness — that check is now dead code and the claims it covered are back to prose", kind)
		}
	}
	if byKind[WitnessGuarded] < 15 {
		t.Errorf("only %d guard-backed inert claims (expected the fs/git/library/replay blocks, ~20) — the strongest witness is draining out of the record", byKind[WitnessGuarded])
	}
	if byKind[WitnessNone] > len(unwitnessedInertClaims) {
		t.Errorf("%d prose-only claims for %d pinned members", byKind[WitnessNone], len(unwitnessedInertClaims))
	}
}

// verifyNarrows checks a "this only undoes that" claim structurally: same
// namespace, a verb pair from the closed table in composition.go, and a twin
// that has itself been considered. Without this, "it can only remove things" is
// a sentence any method can wear.
func verifyNarrows(t *testing.T, method string, claim InertClaim, widens string, halves map[string]bool) {
	t.Helper()
	if widens == "" {
		t.Errorf("%q claims to narrow nothing", method)
		return
	}
	if widens == method {
		t.Errorf("%q claims to be its own undo", method)
		return
	}
	if MissingClassification(widens) {
		t.Errorf("%q claims to undo %q, which capspec does not classify at all", method, widens)
		return
	}
	if _, considered := compositionInert[widens]; !considered && !halves[widens] {
		t.Errorf("%q claims to undo %q, and %q has itself never been considered for composition — the widening direction is the dangerous one, and it is unexamined", method, widens, widens)
	}
	ns := func(m string) string {
		if i := strings.LastIndex(m, "."); i >= 0 {
			return m[:i]
		}
		return m
	}
	verb := func(m string) string {
		if i := strings.LastIndex(m, "."); i >= 0 {
			return m[i+1:]
		}
		return m
	}
	if ns(method) != ns(widens) {
		t.Errorf("%q claims to undo %q, but they are not operations on the same thing (%s vs %s) — a narrowing claim across namespaces is an assertion, not a structure", method, widens, ns(method), ns(widens))
		return
	}
	ok := false
	for _, undo := range inverseVerbs[verb(widens)] {
		if undo == verb(method) {
			ok = true
		}
	}
	if !ok {
		t.Errorf("%q claims to undo %q, and (%s → %s) is not a pair in inverseVerbs. Either it is not an undo, or the table has to say it is — in the open, once, for every method that leans on it.", method, widens, verb(widens), verb(method))
	}
	if !strings.Contains(claim.Reason, widens) {
		t.Errorf("%q rests on being the undo of %q and never names it: %q", method, widens, claim.Reason)
	}
}

// reasonNamesParam requires the sentence to actually point at the parameter its
// witness rests on. Short names are required to be backticked, because "on" and
// "id" match half the English in these paragraphs by accident.
func reasonNamesParam(reason, param string) bool {
	if len(param) <= 4 {
		return strings.Contains(reason, "`"+param+"`")
	}
	return regexp.MustCompile(`\b` + regexp.QuoteMeta(param) + `\b`).MatchString(reason)
}

// TestCompositionRecordNamesOnlyRegisteredCapabilities closes the cheapest way
// in: a method nobody serves.
//
// The forcing function's population comes from capspec's own tables, so adding a
// name to unscopedByDecision invents a capability as far as this file is
// concerned — and pairing that invention with an inert paragraph was enough to
// satisfy every composition check. A composition half and an inert claim must
// both name something a provider actually registers.
func TestCompositionRecordNamesOnlyRegisteredCapabilities(t *testing.T) {
	registered := map[string]bool{}
	for _, m := range capNameRe.FindAllStringSubmatch(string(mustReadRepoFile(t, desktopCapabilitiesSrc...)), -1) {
		registered[m[1]] = true
	}
	hubMain := string(mustReadRepoFile(t, "services", "hub", "cmd", "hub", "main.go"))
	goReg := regexp.MustCompile(`Register(?:Local|LocalIdent|Ident)?\(\s*"([a-zA-Z][\w.]*)"`)
	for _, m := range goReg.FindAllStringSubmatch(hubMain, -1) {
		registered[m[1]] = true
	}
	if len(registered) < 50 {
		t.Fatalf("parsed only %d registered capabilities from the two providers — the registration syntax changed and this guard is comparing against nothing", len(registered))
	}

	names := map[string]bool{}
	for _, c := range Compositions() {
		names[c.A] = true
		names[c.B] = true
	}
	for m := range compositionInert {
		names[m] = true
	}
	for _, name := range sortedFixtureMethods(names) {
		if registered[name] {
			continue
		}
		if _, isTopic := EventTopicCapability(name); isTopic {
			continue // an event topic is delivered, not registered
		}
		t.Errorf("the composition record names %q, and neither hubCapabilities.ts nor cmd/hub/main.go registers it. A record about a capability that does not exist is worse than silence: it reads as coverage.", name)
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
