package main

import (
	"errors"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
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
