package main

import (
	"bytes"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

// A contract fixture with one loader is a dead contract: it looks like a
// cross-language guarantee, it costs the same to maintain, and it guarantees
// nothing — a single implementation trivially agrees with itself. CI cannot
// catch this on its own, because .github/workflows/ci.yml runs one job per
// stack with working-directory set; nothing enumerates contracts/, so a fixture
// added with a Go loader and no TS loader (or vice versa) goes fully green.
//
// This test is the enumeration. It DISCOVERS both sides — the fixtures on disk
// and the files that mention them — rather than carrying a hardcoded table,
// because a hardcoded table would be exactly the drift it exists to prevent.
//
// Placement in cmd/brain is arbitrary; it is where the other cross-repo readers
// already live (config_test.go, configlock_test.go, stores_test.go,
// fsguard_test.go all reach four levels up for contracts/), so it inherits the
// same path idiom and the same skip-if-unreachable posture.

// repoRootRel is relative to this package dir (services/hub/cmd/brain).
const repoRootRel = "../../../.."

// contractSourceExt maps a source-file extension to the language it counts as.
// A fixture needs loaders in at least two DIFFERENT languages: two Go files
// reading the same JSON pin nothing, since they share an implementation.
var contractSourceExt = map[string]string{
	".ts":  "ts",
	".tsx": "ts",
	".go":  "go",
	".rs":  "rust",
}

// contractSkipDirs are directories the repo walk never descends into.
//   - node_modules / target / dist / release / build: dependency and build output,
//     which can hold stale duplicates of both fixtures and loaders.
//   - .git: object store; huge, and pack files can contain anything.
//   - .claude: holds worktrees/ (e.g. .claude/worktrees/electron-43-upgrade), a
//     stale checkout with an OLDER copy of these very files. Counting it would let
//     a fixture look loaded by a copy nobody ships.
var contractSkipDirs = map[string]bool{
	"node_modules": true,
	"target":       true,
	"dist":         true,
	"release":      true,
	"build":        true,
	".git":         true,
	".claude":      true,
}

func TestEveryContractFixtureHasAtLeastTwoLoaders(t *testing.T) {
	repoRoot, contractsDir := contractsDirOrSkip(t)

	fixtures := contractFixtureNames(t, contractsDir)
	if len(fixtures) == 0 {
		// Not a pass. Either every contract was deleted, or the discovery below
		// is broken — both mean this guard is no longer guarding anything.
		t.Fatalf("no *.json fixtures found in %s; contract discovery is broken or the corpus was deleted", contractsDir)
	}

	sources := contractSourceFiles(t, repoRoot)
	// Sanity-check the walk itself: if the skip list ever swallows the repo, every
	// fixture would report zero loaders and the failure message would blame the
	// fixtures instead of the walk.
	byLang := map[string]int{}
	for _, s := range sources {
		byLang[s.lang]++
	}
	for _, lang := range []string{"ts", "go", "rust"} {
		if byLang[lang] == 0 {
			t.Fatalf("repo walk from %s found no %s sources (found %v); the skip list is eating the repo", repoRoot, lang, byLang)
		}
	}

	readme := contractReadme(t, contractsDir)

	for _, fixture := range fixtures {
		t.Run(fixture, func(t *testing.T) {
			var loaders []string
			langs := map[string]bool{}
			needle := []byte(fixture)
			for _, s := range sources {
				if bytes.Contains(s.content, needle) {
					loaders = append(loaders, s.rel)
					langs[s.lang] = true
				}
			}
			sort.Strings(loaders)

			if len(loaders) < 2 {
				t.Errorf("contracts/%s is referenced by %d source file(s) %v — a fixture with fewer than two loaders is a dead contract: it pins nothing and one side can drift freely. Either add the missing loader or delete the fixture.",
					fixture, len(loaders), loaders)
			}
			if len(langs) < 2 {
				t.Errorf("contracts/%s is loaded only from %v (%v) — the point of a contract is that two LANGUAGES agree; same-language loaders share the implementation they are supposed to be checking.",
					fixture, sortedKeys(langs), loaders)
			}

			// The owner table in the README is documentation that goes stale
			// silently. A fixture missing from it has no discoverable owners.
			if !bytes.Contains(readme, needle) {
				t.Errorf("contracts/%s is not mentioned in contracts/README.md — add it to the owner table so its owners and what it guards stay discoverable", fixture)
			}
		})
	}

	// The reverse direction: the README must not advertise fixtures that no
	// longer exist, or a reader trusts a guarantee that was deleted.
	t.Run("README lists no missing fixtures", func(t *testing.T) {
		present := map[string]bool{}
		for _, f := range fixtures {
			present[f] = true
		}
		for _, name := range readmeFixtureMentions(readme) {
			if !present[name] {
				t.Errorf("contracts/README.md documents %q, which does not exist in %s", name, contractsDir)
			}
		}
	})
}

func contractsDirOrSkip(t *testing.T) (repoRoot, contractsDir string) {
	t.Helper()
	repoRoot = filepath.FromSlash(repoRootRel)
	contractsDir = filepath.Join(repoRoot, "contracts")
	if _, err := os.Stat(contractsDir); err != nil {
		// Matches the posture of the other cross-repo readers: the package can be
		// vendored or tested outside the monorepo checkout.
		t.Skipf("contracts dir unreachable from %s: %v", repoRootRel, err)
	}
	return repoRoot, contractsDir
}

func contractFixtureNames(t *testing.T, contractsDir string) []string {
	t.Helper()
	entries, err := os.ReadDir(contractsDir)
	if err != nil {
		t.Skipf("read contracts dir: %v", err)
	}
	var names []string
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue // README.md and any future subdirectories
		}
		names = append(names, e.Name())
	}
	sort.Strings(names)
	return names
}

func contractReadme(t *testing.T, contractsDir string) []byte {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join(contractsDir, "README.md"))
	if err != nil {
		t.Fatalf("read contracts/README.md: %v", err)
	}
	return raw
}

// readmeFixtureMentions pulls every *.json basename the README names, so a
// deleted fixture still advertised in the owner table is caught.
func readmeFixtureMentions(readme []byte) []string {
	seen := map[string]bool{}
	var out []string
	for _, field := range strings.FieldsFunc(string(readme), func(r rune) bool {
		return r == ' ' || r == '\t' || r == '\n' || r == '\r' || r == '`' || r == '|' ||
			r == '(' || r == ')' || r == ',' || r == '"' || r == '\''
	}) {
		field = strings.TrimSuffix(field, ".")
		if !strings.HasSuffix(field, ".json") || strings.ContainsAny(field, "/\\") {
			continue // paths point at fixtures elsewhere (config.yaml tooling, plugin manifests)
		}
		if !seen[field] {
			seen[field] = true
			out = append(out, field)
		}
	}
	sort.Strings(out)
	return out
}

type contractSource struct {
	rel     string // repo-relative, slash-separated, for readable failures
	lang    string
	content []byte
}

// contractSourceFiles walks the repo once and returns every source file in a
// language that can own a contract copy. Reading them all up front keeps the
// per-fixture loop from re-walking the tree for each fixture.
func contractSourceFiles(t *testing.T, repoRoot string) []contractSource {
	t.Helper()
	var out []contractSource
	err := filepath.WalkDir(repoRoot, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			// An unreadable corner of the tree must not fail the whole guard.
			if d != nil && d.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if d.IsDir() {
			if contractSkipDirs[d.Name()] {
				return filepath.SkipDir
			}
			return nil
		}
		lang, ok := contractSourceExt[strings.ToLower(filepath.Ext(d.Name()))]
		if !ok {
			return nil
		}
		if strings.HasSuffix(d.Name(), ".d.ts") {
			return nil // generated declarations, never a loader
		}
		content, readErr := os.ReadFile(path)
		if readErr != nil {
			return nil
		}
		rel, relErr := filepath.Rel(repoRoot, path)
		if relErr != nil {
			rel = path
		}
		out = append(out, contractSource{rel: filepath.ToSlash(rel), lang: lang, content: content})
		return nil
	})
	if err != nil {
		t.Fatalf("walk repo from %s: %v", repoRoot, err)
	}
	return out
}

func sortedKeys(m map[string]bool) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}
