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

// Authenticated agents may select any absolute project. This pins Windows path
// spelling parity both under and outside the manager cwd; the brief filename is
// still derived beneath whichever project the caller selected.
func TestBriefToolsAcceptAmbientWindowsProjectSpellings(t *testing.T) {
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

	for _, ambientRoot := range []string{outside, prefixSibling} {
		variant := windowsManagerPathVariant(ambientRoot)
		for _, tc := range []struct {
			method string
			params map[string]any
		}{
			{"brief.append", map[string]any{"project": variant, "section": "Recently", "line": "ambient update"}},
			{"brief.check", map[string]any{"project": variant}},
			{"brief.archive", map[string]any{"project": variant, "section": "Recently", "count": 1}},
		} {
			if err := call(tc.method, tc.params); err != nil {
				t.Errorf("%s rejected ambient project %s: %v", tc.method, ambientRoot, err)
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
