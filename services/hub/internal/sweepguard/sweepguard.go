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
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

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

// Root walks up from the working directory to the monorepo root. Returns
// ErrNoCheckout only when no ancestor carries the markers.
func Root() (string, error) {
	wd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("getwd: %w", err)
	}
	for dir := wd; ; {
		found := true
		for _, m := range rootMarkers {
			if _, err := os.Stat(filepath.Join(dir, m)); err != nil {
				found = false
				break
			}
		}
		if found {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
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
	// outside the module root. Without the pin, editing hubCapabilities.ts or a
	// contracts/ fixture leaves every guard that reads it printing
	// `ok (cached)` — a pass over bytes it never looked at. See internal/extinput.
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
