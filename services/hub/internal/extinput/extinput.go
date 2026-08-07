// Package extinput makes `go test`'s result cache notice when a file OUTSIDE
// this Go module changes.
//
// Every cross-language guard in this repo reads something the Go module does
// not contain: capspec's detectors parse apps/desktop/src/main/services/
// hubCapabilities.ts, and the corpus loaders parse contracts/*.json. Both live
// above services/hub, and cmd/go's test cache says this about such files
// (cmd/go/internal/test/test.go, computeTestInputsID):
//
//	if a.Package.Root == "" || search.InDir(name, a.Package.Root) == "" {
//		// Do not recheck files outside the module, GOPATH, or GOROOT root.
//		break
//	}
//
// So a plain os.ReadFile of that file is invisible to the cache key. Edit
// hubCapabilities.ts to add an unclassified `env` param to terminals.create,
// run `go test ./...`, and the detector that exists to catch exactly that
// prints `ok (cached)` without ever re-reading the file. Every fix pinned by a
// cross-repo guard is therefore pinned only until the next cached run — which
// is the failure mode this whole round is about: guards that report a pass they
// did not earn.
//
// # The mechanism that does NOT work, and why
//
// This package used to hash the file's bytes and then call os.Getenv on a
// variable whose NAME was that hash, on the theory that `getenv` lines ARE
// recorded and re-hashed. They are re-hashed — but as
//
//	fmt.Fprintf(h, "env %s %x\n", name, hashGetenv(name))
//
// where `name` is the name RECORDED IN THE TESTLOG BY THE PREVIOUS RUN and
// hashGetenv reads the variable's current value. The recorded name still
// carries the OLD content hash and the value is unset in both runs, so the
// recomputed testInputsID is bit-identical no matter what the file now says.
// The name only changes when the test binary RUNS — which is the very thing the
// cache hit is preventing. The pin was a no-op in exactly the case it existed
// for, and `go test ./internal/capspec/` reported `ok (cached)` on a tree that
// `go test -count=1` failed.
//
// # The mechanism that works
//
// computeTestInputsID rejects out-of-root paths LEXICALLY: search.InDir is a
// string-prefix test (str.HasFilePathPrefix), and the recorded name is only
// filepath.Join'd — hence cleaned — when it is RELATIVE. An absolute path that
// still contains ".." reaches the check uncleaned, so
//
//	/repo/services/hub/../../apps/desktop/src/main/services/hubCapabilities.ts
//
// is "inside" the module root as far as InDir is concerned, while the kernel
// resolves it to the real file. cmd/go then calls hashOpen on it — os.Stat
// follows the traversal — and the file's size and mtime land in the cache key.
// [Path] builds that form; [ReadFile] and [ReadDir] read through it, which is
// all it takes, because os.OpenFile calls testlog.Open BEFORE the syscall.
//
// Two consequences worth knowing:
//
//   - A file that does not exist is pinned too, as the error state hashOpen
//     records for a failed stat. That matters as much as the success path: a
//     guard that skips when its cross-repo file is missing would otherwise
//     cache the skip and keep reporting it after the file came back — a guard
//     that switched itself off and stayed off.
//   - hashOpen keys regular files on SIZE and MTIME, not content (and refuses
//     to cache at all while a file is younger than modTimeCutoff = 2s). So an
//     edit that preserves both bytes-length and mtime is invisible. That is
//     precisely the guarantee cmd/go gives its own in-module testdata, so this
//     is parity, not a shortcut — but it is why `make test-hub` and CI still
//     pass -count=1 for the packages that carry cross-repo guards. See the
//     Makefile.
//
// The other half of that belt: cmd/go re-hashes only what a test OPENED.
// Enumerating an out-of-module directory through sweepguard.RepoPath +
// os.ReadDir/filepath.WalkDir pins the files that get read but not the listing,
// so a NEW file appearing in a walked tree is not by itself a cache miss.
// [ReadDir] pins a listing for callers that need one.
//
// Use [ReadFile] wherever a test reads a repo file from outside services/hub.
package extinput

import (
	"fmt"
	"os"
	"path/filepath"
)

// moduleRootDir is resolved at init, from the working directory `go test` gives
// the test binary (always the package's source directory, always inside the
// module). It is captured once on purpose: tests chdir, and a root that moved
// with them would silently produce paths that no longer descend from the root
// cmd/go compares against — the same silent no-op this package exists to kill.
var (
	moduleRootDir string
	moduleRootErr error
)

func init() { moduleRootDir, moduleRootErr = findModuleRoot() }

func findModuleRoot() (string, error) {
	wd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("extinput: getwd: %w", err)
	}
	for dir := wd; ; {
		if info, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil && !info.IsDir() {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("extinput: no go.mod in any ancestor of %s, so there is no module root to pin external inputs against", wd)
		}
		dir = parent
	}
}

// ModuleRoot is the directory holding this module's go.mod: what cmd/go calls
// a.Package.Root, and the prefix every pinned path must start with.
func ModuleRoot() (string, error) { return moduleRootDir, moduleRootErr }

// Path rewrites a path into the form cmd/go's test cache will re-check: the
// module root, then a relative traversal out of it. The result addresses the
// same file as `path` and is a lexical descendant of the module root, which is
// the only property search.InDir tests.
//
// Paths already inside the module root come back cleaned and unchanged in
// meaning; they were never the problem.
func Path(path string) (string, error) {
	if moduleRootErr != nil {
		return "", moduleRootErr
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", fmt.Errorf("extinput: absolute path for %s: %w", path, err)
	}
	rel, err := filepath.Rel(moduleRootDir, abs)
	if err != nil {
		// Different volumes on Windows: no traversal can express it.
		return "", fmt.Errorf("extinput: %s cannot be addressed relative to the module root %s (%w), so a `go test` cache hit could not see it change", abs, moduleRootDir, err)
	}
	// Concatenated, NOT filepath.Join'd: Join cleans, and cleaning the "../"
	// segments away is exactly what makes the path out-of-root again.
	return moduleRootDir + string(filepath.Separator) + rel, nil
}

// ReadFile is os.ReadFile through the pinned path, for inputs outside the
// module. The bytes are identical to os.ReadFile(path); the difference is that
// the read is now part of this package's test cache key.
func ReadFile(path string) ([]byte, error) {
	pinned, err := Path(path)
	if err != nil {
		// Reading anyway would return the right bytes and a cache key that
		// ignores them — a pass nobody earned. Fail instead.
		return nil, err
	}
	data, err := os.ReadFile(pinned)
	if err != nil {
		return nil, pinError(path, pinned, err)
	}
	if err := sameFile(path, pinned); err != nil {
		return nil, err
	}
	return data, nil
}

// ReadDir is os.ReadDir through the pinned path. hashOpen hashes a directory as
// its entry names plus each entry's stat, so a file APPEARING in an
// out-of-module tree becomes a cache miss — which a per-file pin cannot do.
func ReadDir(path string) ([]os.DirEntry, error) {
	pinned, err := Path(path)
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(pinned)
	if err != nil {
		return nil, pinError(path, pinned, err)
	}
	if err := sameFile(path, pinned); err != nil {
		return nil, err
	}
	return entries, nil
}

// pinError keeps the two failures apart. A missing file is the caller's
// problem and is reported as such (and the failed open is itself pinned, so the
// failure will not outlive the fix). A path readable directly but NOT through
// its pinned form means the traversal landed somewhere else — a bind mount, an
// exotic symlink — and that is this package's problem: the read would have
// worked and been invisible to the cache.
func pinError(path, pinned string, err error) error {
	if _, direct := os.Stat(path); direct == nil {
		return fmt.Errorf("extinput: %s is readable but its cache-pinned form %s is not (%w) — reading it directly would work and be invisible to `go test`'s cache, so this is a hard failure, not a fallback", path, pinned, err)
	}
	return err
}

func sameFile(path, pinned string) error {
	di, derr := os.Stat(path)
	pi, perr := os.Stat(pinned)
	if derr != nil || perr != nil || os.SameFile(di, pi) {
		return nil
	}
	return fmt.Errorf("extinput: the cache-pinned path %s does not name the same file as %s — the traversal out of the module root resolves elsewhere, so what this test read and what the cache would re-check are two different files", pinned, path)
}
