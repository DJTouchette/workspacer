//go:build windows

package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A Fleet Manager is itself the one live workspace root; no worker cwd is
// registered here. Its projects inherit that root by containment. This pins the
// exact manager call path that failed when callers used a lower-cased drive and
// slash-separated project path on Windows, and it pins the two denials that
// bound the fix: a directory outside the root, and a SIBLING whose path is the
// root's own path plus more letters.
//
// Both halves are load-bearing and each kills a different mutation.
//   - The three accepts fail the moment canonicalPathsEqual /
//     canonicalPathHasPrefix go back to byte-exact ("==" and strings.HasPrefix):
//     the manager's own workspace, spelled the way a Windows caller spells it,
//     is refused. That false denial is the defect this file exists for.
//   - "FleetManagerOther" is not inside "FleetManager", but it IS prefixed by
//     it. Drop the separator containsPath appends to a non-volume root and the
//     ordinal prefix test says yes, handing one manager's brief tools another
//     workspace. Case-folding the comparison is exactly the change that makes
//     that widening reachable without a `..`, so the deny is asserted here and
//     not left to the corpus.
func TestBriefToolsAcceptWindowsManagerWorkspaceSpellings(t *testing.T) {
	sandbox := t.TempDir()
	setHome(t, filepath.Join(sandbox, "home"))
	setConfigHome(t, filepath.Join(sandbox, "config"))
	managerCwd := filepath.Join(sandbox, "FleetManager")
	project := filepath.Join(managerCwd, "Projects", "Client")
	outside := filepath.Join(sandbox, "Outside")
	// A sibling of the manager root whose path STARTS WITH the manager root's
	// path. filepath.Join is not used for the name for a reason: the string
	// relationship is the test.
	prefixSibling := managerCwd + "Other"
	for _, dir := range []string{managerCwd, project, outside, prefixSibling} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	reg := registryWithCwd(t, managerCwd)
	projectVariant := windowsManagerPathVariant(project)
	call := func(method string, params map[string]any) error {
		body, err := json.Marshal(params)
		if err != nil {
			t.Fatal(err)
		}
		_, err = reg.handle(context.Background(), method, json.RawMessage(body))
		return err
	}

	if err := call("brief.append", map[string]any{"project": projectVariant, "section": "Recently", "line": "manager update"}); err != nil {
		t.Fatalf("brief.append rejected a project under the manager root with equivalent Windows spelling: %v", err)
	}
	if err := call("brief.check", map[string]any{"project": projectVariant}); err != nil {
		t.Fatalf("brief.check rejected a project under the manager root with equivalent Windows spelling: %v", err)
	}
	// COUNT ALONE. archiveOldestEntries refuses count and keep together — they
	// answer different questions and a caller that sent both has not decided
	// which — so sending the pair asserted nothing about containment: the call
	// failed on its arguments before the guard's answer could matter, and the
	// failure read as a rejection.
	if err := call("brief.archive", map[string]any{"project": projectVariant, "section": "Recently", "count": 1}); err != nil {
		t.Fatalf("brief.archive rejected a project under the manager root with equivalent Windows spelling: %v", err)
	}

	for _, outsideRoot := range []string{outside, prefixSibling} {
		variant := windowsManagerPathVariant(outsideRoot)
		for _, tc := range []struct {
			method string
			params map[string]any
		}{
			{"brief.append", map[string]any{"project": variant, "section": "Recently", "line": "must not write"}},
			{"brief.check", map[string]any{"project": variant}},
			{"brief.archive", map[string]any{"project": variant, "section": "Recently", "count": 1}},
		} {
			if err := call(tc.method, tc.params); err == nil || !strings.Contains(err.Error(), refusalText) {
				t.Errorf("%s accepted %s through the manager root: %v", tc.method, outsideRoot, err)
			}
		}
	}
}

// TestWindowsPathContainmentMatchesTheOrdinalContract runs the production
// predicate over windowsContainmentContract, the same vectors the Linux-side
// mutation proof uses. That is what ties the two together: the mutation proof
// shows the vectors kill a byte-exact comparison and a separator-dropping
// prefix test, and this shows the shipping containsPath answers them the way
// the contract says — on the one host where kernel32 can actually be called.
func TestWindowsPathContainmentMatchesTheOrdinalContract(t *testing.T) {
	for _, tc := range windowsContainmentContract {
		if got := containsPath(tc.root, tc.target); got != tc.want {
			t.Errorf("containsPath(%q, %q) = %v, want %v — %s", tc.root, tc.target, got, tc.want, tc.name)
		}
	}
}

func windowsManagerPathVariant(p string) string {
	p = strings.ReplaceAll(p, `\`, "/")
	var b strings.Builder
	b.Grow(len(p))
	for i := 0; i < len(p); i++ {
		c := p[i]
		if c >= 'a' && c <= 'z' {
			b.WriteByte(c - ('a' - 'A'))
		} else if c >= 'A' && c <= 'Z' {
			b.WriteByte(c + ('a' - 'A'))
		} else {
			b.WriteByte(c)
		}
	}
	return b.String()
}
