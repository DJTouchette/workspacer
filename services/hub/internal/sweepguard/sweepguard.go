// Package sweepguard is the machinery a fixture-driven test sweep needs to
// prove it ran.
//
// Every corpus loader in this repo has the same two failure modes, and both
// report a PASS:
//
//  1. The fixture, or the cross-repo twin the guard reads, is not where the
//     loader looked. The loader t.Skipf's ("vendored checkout"), the package is
//     `ok`, and a renamed file has silently deleted a whole guard.
//  2. Every case in the sweep skipped. Each case carries a host requirement
//     (needsSymlinks, needsUnreadableDir, needsHome, posixOnly) and skips
//     itself; a host that fails one of them turns a 107-case corpus into zero
//     executed cases inside a green suite. This has now been found three times:
//     a rootSet oracle whose 8 subtests all skipped, a brain method sweep that
//     ran ZERO deny cases whenever TMPDIR sat under $HOME, and the sessions
//     store's four derived-entry tests.
//
// Both are fixed the same way: the only outcomes a sweep may have are RUN or
// LOUDLY SKIPPED, and something must assert that the number of cases it
// actually executed is non-zero — with denies counted SEPARATELY from allows,
// because a corpus that ran only its allow cases is a corpus that proved a
// guard lets things through and nothing else.
//
// Nothing here imports "testing": the caller owns the failure verb, so the same
// Tally works from a t.Fatal, a t.Error, or a benchmark.
package sweepguard

import (
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/extinput"
)

// Tally counts what a sweep EXECUTED, not what it enumerated. A case is counted
// at the point it has cleared every skip gate and is about to assert — which is
// why Ran is called from inside the subtest body, after the sandbox is built,
// and never from the loop that registers subtests.
type Tally struct {
	Allow   int
	Deny    int
	Other   int
	Skipped int

	skipReasons map[string]int
}

// Ran records one executed case, filed by the fixture's own verdict word
// ("allow"/"accept" vs "deny"/"refuse"), so the two floors can be asserted
// independently.
func (t *Tally) Ran(expect string) {
	switch strings.ToLower(strings.TrimSpace(expect)) {
	case "allow", "accept", "ok", "pass":
		t.Allow++
	case "deny", "refuse", "reject", "fail":
		t.Deny++
	default:
		t.Other++
	}
}

// Skip records a case the sweep did not execute, with the reason. The reasons
// are what makes a floor failure actionable: "0 deny cases (skipped 41:
// needsSymlinks×41)" names the host privilege to fix, where a bare count does
// not.
func (t *Tally) Skip(reason string) {
	t.Skipped++
	if t.skipReasons == nil {
		t.skipReasons = map[string]int{}
	}
	key := strings.TrimSpace(reason)
	if key == "" {
		key = "unspecified"
	}
	t.skipReasons[key]++
}

// Executed is the total number of cases that asserted anything.
func (t *Tally) Executed() int { return t.Allow + t.Deny + t.Other }

// Require reports an error unless the sweep executed at least minAllow allow
// cases and minDeny deny cases. `what` names the sweep in the failure.
func (t *Tally) Require(what string, minAllow, minDeny int) error {
	var missing []string
	if t.Allow < minAllow {
		missing = append(missing, fmt.Sprintf("%d allow cases (want >= %d)", t.Allow, minAllow))
	}
	if t.Deny < minDeny {
		missing = append(missing, fmt.Sprintf("%d deny cases (want >= %d)", t.Deny, minDeny))
	}
	if len(missing) == 0 {
		return nil
	}
	return fmt.Errorf(
		"%s executed %s — a sweep that ran none of a verdict class asserted nothing about it and is a PASS that guards nothing%s",
		what, strings.Join(missing, " and "), t.skipSuffix())
}

// RequireBoth is the usual floor: at least one allow and at least one deny.
func (t *Tally) RequireBoth(what string) error { return t.Require(what, 1, 1) }

// Enumerated is every case the sweep REACHED — executed plus skipped. It is the
// host-independent number: a case that skips for want of symlink privilege
// still counts here, and a case deleted from the fixture does not.
func (t *Tally) Enumerated() int { return t.Executed() + t.Skipped }

// RequireCorpus is the floor with a RATCHET, and it is what a fixture-driven
// sweep must use instead of RequireBoth.
//
// RequireBoth is a floor of ONE. It was written against the failure it had in
// front of it — a sweep that executed NOTHING — and it does not notice the
// slower version of the same thing: a corpus of 79 executed cases that shrinks
// to 2 (a bad merge, a `continue` added to the group filter, a fixture edit that
// drops an array) keeps one allow and one deny and stays green forever. The
// floor has to be pinned near where the sweep actually is.
//
// minEnumerated is checked against Enumerated, NOT against Executed, because
// execution is host-dependent (a machine without symlink privilege legitimately
// skips half the containment corpus) while enumeration is not: it is a property
// of the fixture and the loader's filter, identical on every host. So this one
// number catches a shrinking corpus everywhere, including on the hosts that skip
// the most — which are exactly the hosts where the verdict floors go slack.
//
// minAllow/minDeny stay on Executed, and stay small: they answer "did this host
// prove anything about each verdict class", which is a different question.
func (t *Tally) RequireCorpus(what string, minEnumerated, minAllow, minDeny int) error {
	if got := t.Enumerated(); got < minEnumerated {
		return fmt.Errorf(
			"%s reached %d cases but the floor is %d — the corpus SHRANK (a fixture edit, a group filter, a bad merge); "+
				"this count is host-independent, so it is not a skip. Restore the cases, or lower the floor deliberately and say why%s",
			what, got, minEnumerated, t.skipSuffix())
	}
	return t.Require(what, minAllow, minDeny)
}

// RequireEvery is the floor for a sweep with no verdict column (a slug corpus, a
// normalization table): `min` cases must have EXECUTED, not merely been
// enumerated. It is the strict form — use it wherever nothing in the block is
// host-gated, because there a case that did not run is a loader bug and not a
// machine, and the enumerated floor would be satisfied by a sweep that skipped
// everything.
func (t *Tally) RequireEvery(what string, min int) error {
	if t.Executed() >= min {
		return nil
	}
	return fmt.Errorf(
		"%s executed %d of a floor of %d cases — nothing in this block is host-gated, so a case that did not assert is a loader bug or a corpus that shrank%s",
		what, t.Executed(), min, t.skipSuffix())
}

// --- host-gated groups ------------------------------------------------------

// GateCounter is the floor for a group of HAND-WRITTEN tests gated on a host
// privilege (`if err := os.Symlink(...); err != nil { t.Skipf(...) }`), where
// there is no fixture, no verdict column and nothing to tally — only "did these
// run at all".
//
// It is the Go twin of apps/desktop/tests/support/sweepTally.ts's gatedIt +
// itRanEveryGatedTest, and its absence was the hole: the TypeScript side has had
// this shape since the sessions-store finding, while the Go side had Tally only.
// Tally covers fixture sweeps; NOTHING covered the 23 hand-written Go tests that
// each t.Skipf themselves whole on a host that cannot create a symlink. Simulate
// such a host (WKS_TEST_NO_SYMLINKS=1) and cmd/brain still printed `ok`.
//
// Two rules, both learned from the counters this replaces:
//
//   - Ran is called from the test BODY, past every skip gate. A counter
//     incremented in the loop that registers subtests counts enumeration, not
//     execution, and reports a full house for a run in which every subtest
//     skipped.
//   - The count is keyed by test name and compared EXACTLY. A group that
//     quietly shrinks from four tests to one is the same hole arriving more
//     slowly, so a missing name and an unexpected name both fail.
type GateCounter struct {
	name     string
	expected int

	mu      sync.Mutex
	ran     map[string]bool
	skipped map[string]string
}

var (
	gateMu    sync.Mutex
	gateOrder []*GateCounter
)

// Gate declares a group of host-gated tests and registers it with the process,
// so RunGates can fail the package even when the group's own floor test is
// itself skipped or deleted. `expected` is how many distinct tests call Ran.
func Gate(name string, expected int) *GateCounter {
	g := &GateCounter{name: name, expected: expected, ran: map[string]bool{}, skipped: map[string]string{}}
	gateMu.Lock()
	gateOrder = append(gateOrder, g)
	gateMu.Unlock()
	return g
}

// Ran records that the named test got past its host gate and is asserting.
// Callers pass the test name; recording a SET rather than a count keeps a test
// that creates three symlinks from counting as three.
func (g *GateCounter) Ran(test string) {
	g.mu.Lock()
	defer g.mu.Unlock()
	g.ran[test] = true
	delete(g.skipped, test)
}

// Skip records that the named test could not run, and why.
//
// Last call wins, in BOTH directions, and the Skip-after-Ran direction is the
// one that matters: a test that clears the symlink gate and then skips for some
// other reason (no git, a filename this filesystem will not hold) asserted
// nothing, so it must not stay counted as run. Callers arm that with a
// t.Cleanup that re-reports the skip.
func (g *GateCounter) Skip(test, reason string) {
	g.mu.Lock()
	defer g.mu.Unlock()
	delete(g.ran, test)
	if reason = strings.TrimSpace(reason); reason == "" {
		reason = "unspecified"
	}
	g.skipped[test] = reason
}

// Count reports how many distinct tests in the group executed.
func (g *GateCounter) Count() int {
	g.mu.Lock()
	defer g.mu.Unlock()
	return len(g.ran)
}

// Name is the group's label, for the meta-guard and for failure text.
func (g *GateCounter) Name() string { return g.name }

// Require reports an error unless every declared test in the group executed.
func (g *GateCounter) Require() error {
	g.mu.Lock()
	defer g.mu.Unlock()
	if len(g.ran) == g.expected {
		return nil
	}
	detail := ""
	if len(g.skipped) > 0 {
		names := make([]string, 0, len(g.skipped))
		for n, r := range g.skipped {
			names = append(names, fmt.Sprintf("%s (%s)", n, r))
		}
		sort.Strings(names)
		detail = "; skipped: " + strings.Join(names, ", ")
	}
	if len(g.ran) > g.expected {
		return fmt.Errorf("%s executed %d tests but declares %d — a test was ADDED to the group without raising the count, so the floor no longer knows what full looks like%s",
			g.name, len(g.ran), g.expected, detail)
	}
	return fmt.Errorf("%s executed %d of %d host-gated tests. These are the ONLY oracle for what they cover, so a host that cannot run them must be RED, not a green package with a skip count nobody reads%s",
		g.name, len(g.ran), g.expected, detail)
}

// RegisteredGates returns every gate declared in this process, in declaration
// order.
func RegisteredGates() []*GateCounter {
	gateMu.Lock()
	defer gateMu.Unlock()
	out := make([]*GateCounter, len(gateOrder))
	copy(out, gateOrder)
	return out
}

// RunGates is the TestMain body for a package that declares host gates:
//
//	func TestMain(m *testing.M) { os.Exit(sweepguard.RunGates(m)) }
//
// The floor has to live HERE and not only in a test function, because the whole
// failure mode is a package that reports `ok` while a group of tests skipped
// itself out of existence — and a floor test can be filtered out, renamed away,
// or (as happened) simply never written. TestMain runs whatever else does not.
//
// It stands down when -run/-bench narrows the suite, since a filtered run is
// meant to execute a subset and a floor over one would be noise, and when the
// suite already failed, since a cascade of gate failures buries the real error.
func RunGates(m interface{ Run() int }) int {
	// Thin wrapper: all enforcement lives in `enforceGates`, which is exercised
	// directly by TestRunGatesEnforcesTheFloor. Without that seam the floor's own
	// body had no mutation-killing test — a refactor that returned early (or a
	// dropped os.Exit) defeated every host gate while CI stayed green, because the
	// only oracle for "the gates were checked" was TestMain running in production.
	if verbose() {
		for _, g := range RegisteredGates() {
			fmt.Fprintf(os.Stderr, "gate: %s\n", g)
		}
	}
	return enforceGates(m.Run(), filtered(), RegisteredGates())
}

// enforceGates is the decision core of RunGates, factored out so the floor can be
// mutation-tested with a controlled gate set instead of process-global state.
// Returns the exit code: the suite's own `code` when it already failed, was
// filtered to a subset, or every gate ran; otherwise 1, having printed which
// gate groups did not run.
func enforceGates(code int, filtered bool, gates []*GateCounter) int {
	if code != 0 || filtered {
		return code
	}
	var problems []string
	for _, g := range gates {
		if err := g.Require(); err != nil {
			problems = append(problems, err.Error())
		}
	}
	if len(problems) == 0 {
		return code
	}
	fmt.Fprintf(os.Stderr, "\nFAIL: %d host-gated test group(s) did not run:\n", len(problems))
	for _, p := range problems {
		fmt.Fprintf(os.Stderr, "  - %s\n", p)
	}
	return 1
}

// String is what -v prints for a gate, so a group that is shrinking is visible
// before it reaches zero.
func (g *GateCounter) String() string {
	g.mu.Lock()
	defer g.mu.Unlock()
	return fmt.Sprintf("%s: %d of %d ran, %d skipped", g.name, len(g.ran), g.expected, len(g.skipped))
}

func verbose() bool {
	f := flag.Lookup("test.v")
	return f != nil && f.Value.String() != "" && f.Value.String() != "false"
}

// filtered reports whether this run was narrowed with -run or -bench, in which
// case "not every gated test executed" is the point of the invocation.
func filtered() bool {
	for _, name := range []string{"test.run", "test.bench"} {
		if f := flag.Lookup(name); f != nil && f.Value.String() != "" {
			return true
		}
	}
	return false
}

// RequireDeny is the floor for a sweep that is deny-only by construction — a
// handler probe fed the corpus's refusals, say. It still has to run one.
func (t *Tally) RequireDeny(what string) error { return t.Require(what, 0, 1) }

func (t *Tally) skipSuffix() string {
	if t.Skipped == 0 {
		return ""
	}
	return fmt.Sprintf("; %d case(s) skipped: %s", t.Skipped, t.reasonList())
}

func (t *Tally) reasonList() string {
	keys := make([]string, 0, len(t.skipReasons))
	for k := range t.skipReasons {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	parts := make([]string, 0, len(keys))
	for _, k := range keys {
		parts = append(parts, fmt.Sprintf("%s×%d", k, t.skipReasons[k]))
	}
	return strings.Join(parts, ", ")
}

// String is what a loader logs on success, so a sweep that is shrinking is
// visible in `go test -v` before it reaches zero.
func (t *Tally) String() string {
	return fmt.Sprintf("executed %d cases (%d allow, %d deny, %d other)%s",
		t.Executed(), t.Allow, t.Deny, t.Other, t.skipSuffix())
}

// --- the cross-repo reader --------------------------------------------------

// ErrNoCheckout means the monorepo working tree is not above this package: the
// module was vendored or extracted on its own. It is the ONE situation in which
// skipping a cross-repo cross-check is honest, and it is distinguishable from
// "the checkout is right here and the file moved", which must fail.
var ErrNoCheckout = errors.New("not a workspacer monorepo checkout")

// rootMarkers identify the monorepo root. Both are deliberately things no
// contract test owns: a marker under contracts/ or apps/ would let deleting the
// very thing under test read as "vendored" and skip.
var rootMarkers = []string{
	filepath.Join("services", "hub", "go.mod"),
	"Makefile",
}

// Root walks up from the working directory to the monorepo root.
//
// It returns ErrNoCheckout — the error every caller treats as an honest skip —
// ONLY when no ancestor carries any marker at all. An ancestor carrying SOME of
// them is the dangerous middle case and gets a hard error instead: `Makefile` is
// a marker, so renaming it turned a live monorepo into "not a checkout" and
// silently stood down the ~17 cross-repo guards that read the other stack's
// source through ReadRepoFile. Every one of them still printed ok. A partial
// match is a repo that MOVED A MARKER, and that must be as loud as a moved
// fixture, which is the whole point of the ErrNoCheckout/plain-error split.
func Root() (string, error) {
	wd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("getwd: %w", err)
	}
	// The best partial match seen on the way up, kept for the failure text.
	bestDir, bestHave, bestMissing := "", []string(nil), []string(nil)
	for dir := wd; ; {
		var have, missing []string
		for _, m := range rootMarkers {
			if _, err := os.Stat(filepath.Join(dir, m)); err != nil {
				missing = append(missing, m)
			} else {
				have = append(have, m)
			}
		}
		if len(missing) == 0 {
			return dir, nil
		}
		if len(have) > len(bestHave) {
			bestDir, bestHave, bestMissing = dir, have, missing
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			if bestDir != "" {
				return "", fmt.Errorf(
					"%s carries %v but NOT %v — this looks like the workspacer monorepo with a root marker renamed or deleted. "+
						"That is not a vendored checkout and must not be treated as one: every cross-repo guard keys off Root(), "+
						"so answering ErrNoCheckout here would turn them all into silent skips. Restore the marker, or update sweepguard.rootMarkers",
					bestDir, bestHave, bestMissing)
			}
			return "", fmt.Errorf("%w: no ancestor of %s carries %v", ErrNoCheckout, wd, rootMarkers)
		}
		dir = parent
	}
}

// ReadRepoFile reads a file addressed from the monorepo root. The error it
// returns is the whole point: it wraps ErrNoCheckout only when the checkout
// itself is absent. A missing file inside a present checkout comes back as a
// plain error, which the caller must treat as a FAILURE — that is a rename or a
// deletion, and skipping it is how a cross-repo guard disappears without anyone
// seeing red.
func ReadRepoFile(parts ...string) ([]byte, error) {
	root, err := Root()
	if err != nil {
		return nil, err
	}
	full := filepath.Join(append([]string{root}, parts...)...)
	// extinput.ReadFile, not os.ReadFile: every file this function returns lives
	// ABOVE services/hub, and cmd/go's test cache does not re-check inputs
	// outside the module root. Read directly, editing hubCapabilities.ts or a
	// contracts/ fixture leaves every guard that reads it printing
	// `ok (cached)` — a pass over bytes it never looked at. extinput reads the
	// same file through a path that still descends lexically from the module
	// root, which is the only thing cmd/go's search.InDir check tests, so the
	// read lands in the cache key. See internal/extinput, and note that this
	// costs the caller nothing: the bytes are what os.ReadFile would return.
	data, err := extinput.ReadFile(full)
	if err != nil {
		return nil, fmt.Errorf("the monorepo checkout is at %s but %s is not readable (%w) — this cross-repo guard reads a file that moved or was deleted; a skip here would silently delete the guard", root, full, err)
	}
	return data, nil
}

// RepoPath joins root-relative parts, for the readers that need the path rather
// than the bytes.
func RepoPath(parts ...string) (string, error) {
	root, err := Root()
	if err != nil {
		return "", err
	}
	return filepath.Join(append([]string{root}, parts...)...), nil
}
