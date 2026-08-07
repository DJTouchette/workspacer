package main

import (
	"errors"
	"regexp"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
	yaml "gopkg.in/yaml.v3"
)

// mustReadRepoFile reads a file the guard calling it cannot do its job without:
// a contracts/ fixture, or the desktop twin a cross-language cross-check
// compares against.
//
// Every reader of those files used to be written the same way — os.ReadFile,
// and t.Skipf on any error, with a comment about vendored or hub-only
// checkouts. That posture collapses two very different situations into the same
// silent green:
//
//   - the module really was extracted on its own, where a skip is honest; and
//   - the checkout is right here and the file was RENAMED, MOVED or DELETED,
//     where the skip quietly deletes the guard. A path typo in the "../../../.."
//     prefix does it too, and nothing about a skipped test says so: `go test`
//     prints `ok`.
//
// sweepguard.Root tells them apart by looking for the monorepo's own markers,
// neither of which is a file any contract test owns. Only the first case skips;
// the second is a FAILURE, which is what it always should have been.
func mustReadRepoFile(t *testing.T, parts ...string) []byte {
	t.Helper()
	data, err := sweepguard.ReadRepoFile(parts...)
	if err == nil {
		return data
	}
	if errors.Is(err, sweepguard.ErrNoCheckout) {
		t.Skipf("not a monorepo checkout, so this cross-repo cross-check has nothing to read: %v", err)
	}
	t.Fatalf("%v", err)
	return nil
}

// mustRepoPath is mustReadRepoFile for the guards that need the path rather
// than the bytes (a directory to enumerate, a file to stat).
func mustRepoPath(t *testing.T, parts ...string) string {
	t.Helper()
	p, err := sweepguard.RepoPath(parts...)
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout: %v", err)
		}
		t.Fatalf("%v", err)
	}
	return p
}

// TestTheTestCacheBeltIsInCIAndTheMakefile keeps the second belt on every
// cross-repo guard in this module.
//
// The first belt is internal/extinput: cmd/go's test cache drops inputs outside
// the module root from the cache key, so every reader above services/hub —
// hubCapabilities.ts, contracts/*.json, ci.yml, the Makefile — used to be
// invisible to it, and `go test ./internal/capspec/` printed `ok (cached)` on a
// tree that `go test -count=1` failed. extinput fixes that by reading through a
// path that still descends LEXICALLY from the module root, which is what
// search.InDir tests.
//
// That belt is exact about what it covers: cmd/go hashes a regular file's SIZE
// and MTIME, not its bytes, and it only hashes what a test actually OPENED — so
// a new file appearing in a directory this suite merely enumerates
// (sweepguard.RepoPath + os.ReadDir / filepath.WalkDir) is not a cache miss.
// It also rests on cmd/go internals that are free to change; when they do, the
// failure mode is silence, which is the failure mode this whole round exists to
// remove. -count=1 does not care about any of that, and the module's tests run
// uncached in about seven seconds.
//
// This asserts the shape (a `go test` that carries -count=1), not the exact
// line, so both can be reorganised.
func TestTheTestCacheBeltIsInCIAndTheMakefile(t *testing.T) {
	t.Run("Makefile test-hub", func(t *testing.T) {
		raw := mustReadRepoFile(t, "Makefile")
		recipe := makeRecipe(t, string(raw), "test-hub")
		if !goTestRe.MatchString(recipe) {
			t.Fatalf("the Makefile's test-hub recipe no longer runs `go test`:\n%s", recipe)
		}
		if !strings.Contains(recipe, "-count=1") {
			t.Errorf("`make test-hub` runs the Go suite WITHOUT -count=1:\n%s\n— cmd/go's test cache ignores inputs outside the module root, and this suite's cross-repo guards read nothing but such inputs; without the flag a local `make test-hub` can report a pass over a hubCapabilities.ts or contracts/ fixture it never re-read", recipe)
		}
	})

	// CI matters more than the Makefile: actions/setup-go restores
	// ~/.cache/go-build, which is where the test RESULT cache lives, so a green
	// CI run can be served from a previous run's results on a tree whose
	// cross-repo inputs moved underneath it.
	t.Run("ci.yml go test steps", func(t *testing.T) {
		raw := mustReadRepoFile(t, ".github", "workflows", "ci.yml")
		var wf struct {
			Jobs map[string]struct {
				Steps []struct {
					Run string `yaml:"run"`
				} `yaml:"steps"`
			} `yaml:"jobs"`
		}
		if err := yaml.Unmarshal(raw, &wf); err != nil {
			t.Fatalf("parse ci.yml: %v", err)
		}
		if len(wf.Jobs) == 0 {
			t.Fatal("ci.yml decoded to zero jobs — the parse broke and this guard is guarding nothing")
		}
		checked := 0
		for name, job := range wf.Jobs {
			for _, step := range job.Steps {
				line := strings.TrimSpace(step.Run)
				// Word-anchored: "cargo test" ends in "go test", and matching
				// it made this guard demand -count=1 of the Rust jobs.
				if !goTestRe.MatchString(line) {
					continue
				}
				checked++
				if !strings.Contains(line, "-count=1") {
					t.Errorf("ci.yml job %q runs %q without -count=1 — setup-go restores the go-build cache between runs, and cmd/go's test cache key ignores every out-of-module file this suite's cross-repo guards read", name, line)
				}
			}
		}
		if checked == 0 {
			t.Fatal("no ci.yml step runs `go test` at all — this guard asserted nothing, and neither does CI")
		}
	})
}

// goTestRe matches an invocation of the go tool's test subcommand, and nothing
// whose command merely ENDS in "go test" — cargo does.
var goTestRe = regexp.MustCompile(`(^|[\s;&|(])go\s+test\b`)

// makeRecipe returns the recipe lines of a make target: the tab-indented block
// after `<target>:`. Blank lines and comments end nothing; a non-indented line
// does.
func makeRecipe(t *testing.T, makefile, target string) string {
	t.Helper()
	lines := strings.Split(makefile, "\n")
	for i, line := range lines {
		if !strings.HasPrefix(line, target+":") {
			continue
		}
		var recipe []string
		for _, next := range lines[i+1:] {
			if strings.HasPrefix(next, "\t") {
				recipe = append(recipe, next)
				continue
			}
			if strings.TrimSpace(next) == "" {
				continue
			}
			break
		}
		if len(recipe) == 0 {
			t.Fatalf("make target %q has an empty recipe", target)
		}
		return strings.Join(recipe, "\n")
	}
	t.Fatalf("no %q target in the root Makefile — `make test-hub` is how this repo's Go suite is run, so either it was renamed (update this guard) or the entry point is gone", target)
	return ""
}
