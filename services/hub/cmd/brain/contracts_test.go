package main

import (
	"bytes"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/extinput"
	yaml "gopkg.in/yaml.v3"
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
// fsguard_test.go all reach four levels up for contracts/). It does NOT inherit
// their old skip-if-unreachable posture: see mustReadRepoFile.

// (There is deliberately no "../../../.." repo-root constant here any more. The
// root comes from sweepguard.Root, and every file below it is read through
// extinput, because a path that escapes the module root is exactly what cmd/go
// drops from the test cache key.)

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
	repoRoot, contractsDir := contractsDir(t)

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
			// A LOADER is a test. An implementation file that names the fixture
			// in a comment documents it; it does not hold anything to it.
			var loaders, mentions []string
			langs := map[string]bool{}
			needle := []byte(fixture)
			for _, s := range sources {
				if !bytes.Contains(s.content, needle) {
					continue
				}
				if !s.isTest {
					mentions = append(mentions, s.rel)
					continue
				}
				loaders = append(loaders, s.rel)
				langs[s.lang] = true
			}
			sort.Strings(loaders)
			sort.Strings(mentions)

			if len(loaders) < 2 {
				t.Errorf("contracts/%s is loaded by %d TEST file(s) %v (non-test mentions: %v) — a fixture with fewer than two loaders is a dead contract: it pins nothing and one side can drift freely. Either add the missing loader or delete the fixture.",
					fixture, len(loaders), loaders, mentions)
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

// contractsDir locates contracts/ from the monorepo root. It FAILS when the
// directory is missing inside a real checkout and skips only when there is no
// checkout at all: "the package can be vendored" was the rationale for skipping
// on any error, and it made the enumeration below — the guard that notices a
// fixture nobody loads — vanish silently the moment the relative prefix or the
// directory name drifted.
func contractsDir(t *testing.T) (repoRoot, contractsDir string) {
	t.Helper()
	repoRoot = mustRepoPath(t)
	contractsDir = filepath.Join(repoRoot, "contracts")
	if _, err := os.Stat(contractsDir); err != nil {
		t.Fatalf("the monorepo checkout is at %s but %s is not there (%v) — every contract fixture in this repo lives in it, so this is a deletion or a rename, not a vendored checkout", repoRoot, contractsDir, err)
	}
	return repoRoot, contractsDir
}

func contractFixtureNames(t *testing.T, contractsDir string) []string {
	t.Helper()
	// extinput.ReadDir, not os.ReadDir: this LISTING is the guard's input — the
	// whole test is "every fixture in contracts/ has two loaders" — and cmd/go
	// only re-hashes what a test opened through a path that lexically descends
	// from the module root. Read straight, adding a fixture nobody loads is not
	// a cache miss, and the guard written to catch exactly that reports
	// `ok (cached)`. hashOpen hashes a directory as its entry names plus each
	// entry's stat, so through extinput a new fixture IS a miss.
	entries, err := extinput.ReadDir(contractsDir)
	if err != nil {
		t.Fatalf("read contracts dir %s: %v — an unreadable corpus directory is a broken guard, not a reason to pass", contractsDir, err)
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
	// mustReadRepoFile: contracts/README.md is above the module root, so an
	// os.ReadFile of it never entered cmd/go's test cache key. Measured: touch
	// contracts/README.md and `go test ./...` reported EVERY package cached,
	// this one included — so the owner table could drift from the fixtures it
	// documents and this guard would keep printing a pass it did not earn.
	return mustReadRepoFile(t, "contracts", "README.md")
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
	// isTest marks a file that can actually LOAD a fixture: a Go _test.go, a
	// *.test.ts / *.spec.ts, or a Rust file carrying a #[cfg(test)] module.
	// Without this the guard counted any file whose CONTENT mentioned the
	// fixture basename — including the implementations that merely cite it in a
	// doc comment. Two such comments, in two languages, satisfied both the
	// "fewer than two loaders" and the "fewer than two languages" arms, so a
	// fixture NO TEST LOADS passed; and every real loader of
	// path-containment-cases.json could have been deleted while eight
	// implementation files kept it looking covered.
	isTest bool
}

// isContractTestFile classifies a source file as a test by the convention its
// language uses. Rust is the odd one: its unit tests live inside the
// implementation file behind #[cfg(test)], so the name cannot decide it.
func isContractTestFile(name, lang string, content []byte) bool {
	switch lang {
	case "go":
		return strings.HasSuffix(name, "_test.go")
	case "ts":
		return strings.Contains(name, ".test.") || strings.Contains(name, ".spec.")
	case "rust":
		return bytes.Contains(content, []byte("#[cfg(test)]"))
	}
	return false
}

// contractSourceFiles walks the repo once and returns every source file in a
// language that can own a contract copy. Reading them all up front keeps the
// per-fixture loop from re-walking the tree for each fixture.
func contractSourceFiles(t *testing.T, repoRoot string) []contractSource {
	t.Helper()
	var out []contractSource
	// walkPinned, not filepath.WalkDir: an unreadable corner of the tree is
	// still skipped, but the directory LISTINGS now enter cmd/go's test cache
	// key. Without that, a fixture's second loader could be DELETED — the file
	// simply stops appearing — and this guard, whose whole job is counting
	// loaders, would print `ok (cached)`.
	err := walkPinned(repoRoot, func(path string, d os.DirEntry) error {
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
		// extinput.ReadFile, not os.ReadFile. Most of this walk is OUTSIDE the
		// Go module (apps/desktop's TypeScript, apps/tui and services/claudemon's
		// Rust), and those are precisely the second and third loaders this guard
		// exists to count. Read straight, they are absent from cmd/go's test
		// cache key: measured, touching apps/tui/src/config.rs left every
		// package cached, so deleting a fixture's only Rust loader would keep
		// printing `ok (cached)`. In-module files come back through the same
		// call unchanged — extinput cleans a path it does not have to escape.
		content, readErr := extinput.ReadFile(path)
		if readErr != nil {
			return nil
		}
		rel, relErr := filepath.Rel(repoRoot, path)
		if relErr != nil {
			rel = path
		}
		out = append(out, contractSource{
			rel: filepath.ToSlash(rel), lang: lang, content: content,
			isTest: isContractTestFile(d.Name(), lang, content),
		})
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

// ---------------------------------------------------------------------------
// The containment corpus on Windows.

// TestCorpusRunsOnWindowsInCI asserts that some CI job actually runs the two
// suites that own the Windows-only branches of path containment, on Windows.
//
// This is a test about a workflow file because the gap was a workflow file.
// Every job in ci.yml was `runs-on: ubuntu-latest`; only release.yml touches
// windows-latest, and that leg builds and signs an installer rather than
// running a suite. So the Windows arms of all three copies — pathConfinement.ts
// splitAbsolute's drive/UNC regex pair, fsguard.go splitPath's VolumeName plus
// drive-relative reject, and bus/policy.go splitVolume's hand-rolled scanner,
// three DIFFERENT parsers of the same thing — were executed by nothing at test
// time. All three could be broken so that a drive-relative "C:foo" (which
// resolves against whatever directory the process is on, on drive C) counts as
// absolute, and both full suites stayed green on Linux. That is precisely the
// check-path/opened-path split BINDING DECISION 2 exists to close: the guard
// canonicalizes against a volume root while the handler opens against the
// per-drive cwd.
//
// The corpus cannot close this on its own — its cases run wherever the process
// runs, and every runner is Linux — so the job IS the fix, and this is what
// keeps the job from quietly disappearing. It asserts the shape (a
// windows-latest job that runs `go test` and a vitest invocation naming the
// containment suites), not the exact YAML, so the job can be reorganised.
func TestCorpusRunsOnWindowsInCI(t *testing.T) {
	// FAILS if ci.yml moved: this test is the ONLY thing keeping the
	// windows-latest job alive, and the Windows job is the only execution the
	// three volume-prefix parsers ever get.
	raw := mustReadRepoFile(t, ".github", "workflows", "ci.yml")
	var wf struct {
		Jobs map[string]struct {
			RunsOn string `yaml:"runs-on"`
			Steps  []struct {
				Name string `yaml:"name"`
				Run  string `yaml:"run"`
			} `yaml:"steps"`
		} `yaml:"jobs"`
	}
	if err := yaml.Unmarshal(raw, &wf); err != nil {
		t.Fatalf("parse ci.yml: %v", err)
	}
	if len(wf.Jobs) == 0 {
		t.Fatal("ci.yml decoded to zero jobs — the parse broke and this guard is guarding nothing")
	}

	var goOnWindows, tsOnWindows string
	for name, job := range wf.Jobs {
		if !strings.Contains(job.RunsOn, "windows") {
			continue
		}
		for _, step := range job.Steps {
			if strings.Contains(step.Run, "go test") {
				goOnWindows = name
			}
			// The desktop suites that own pathConfinement.ts. Either the whole
			// suite or the confinement files by name counts.
			if strings.Contains(step.Run, "vitest") &&
				(strings.Contains(step.Run, "pathConfinement") || !strings.Contains(step.Run, ".test.ts")) {
				tsOnWindows = name
			}
		}
	}
	if goOnWindows == "" {
		t.Error("no windows-latest job in .github/workflows/ci.yml runs `go test` — fsguard.go's and bus/policy.go's Windows volume-prefix branches (and every posixOnly/needsSymlinks flag the corpus spends on them) are executed by nothing, on any machine anyone runs")
	}
	if tsOnWindows == "" {
		t.Error("no windows-latest job in .github/workflows/ci.yml runs vitest over pathConfinement — splitAbsolute's drive/UNC regexes are the third hand-rolled volume parser and the only one with no Windows execution at all")
	}
}
