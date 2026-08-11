package main

// The `spawnCwds` block of contracts/path-containment-cases.json: the ONE
// normalization a caller-supplied spawn / terminal working directory gets.
//
// agents.spawn and terminals.create are both registered by BOTH providers, and
// capspec lists both under unscopedByDecision — the cwd is the point of the
// call, not something the path guard confines. That makes the string each
// provider hands the daemon the entire contract, and the two disagreed on five
// of the eight spellings a probe tried: this side tilde-expanded, trimmed and
// stripped trailing slashes; the desktop existence-checked and fell back to
// $HOME. Nothing in either suite noticed, because each side tested its own rule.
//
// The tilde half is not cosmetic. A session's stored cwd is what agentCwds()
// feeds into workspaceRoots(), so `agents.spawn {"cwd":"~"}` turned the whole
// home tree into an fs.* root here and into nothing at all on the other
// provider — precisely the allowed-by-one/denied-by-the-other split BINDING
// DECISION 1 exists to close.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

type spawnCwdCase struct {
	In  string `json:"in"`
	Out string `json:"out"`
	Why string `json:"why"`
}

func loadSpawnCwdCases(t *testing.T) []spawnCwdCase {
	t.Helper()
	path := filepath.Join("contracts", "path-containment-cases.json")
	// The fixture is the test. Skipping when it is unreadable is a green PASS
	// over zero cases — see mustReadRepoFile.
	raw := mustReadRepoFile(t, "contracts", "path-containment-cases.json")
	var fx struct {
		SpawnCwds struct {
			Cases []spawnCwdCase `json:"cases"`
		} `json:"spawnCwds"`
	}
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", path, err)
	}
	if len(fx.SpawnCwds.Cases) == 0 {
		t.Fatal("the spawnCwds block is empty — this loader is holding nothing to anything")
	}
	return fx.SpawnCwds.Cases
}

// spawnCwdFloor is the block's size today; `len(...) == 0` is met by a block
// down to one case.
const spawnCwdFloor = 14

func TestSpawnCwdNormalizationContractCases(t *testing.T) {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		t.Skip("this process has no resolvable home directory")
	}
	cases := loadSpawnCwdCases(t)

	// sawTilde is counted in the BODY. Registered-and-never-run is the failure
	// this whole family keeps re-learning, and a tilde vector that was
	// enumerated but never executed satisfies nothing: the guard it stands for
	// (BINDING DECISION 1 on this seam) would be unpinned with the flag still
	// true. Same for the tally.
	sawTilde := false
	var tally sweepguard.Tally
	for _, c := range cases {
		t.Run(c.In, func(t *testing.T) {
			if strings.HasPrefix(c.In, "~") {
				sawTilde = true
			}
			tally.Ran("other")
			want := strings.ReplaceAll(c.Out, "${HOME}", home)
			if i := strings.Index(want, "${"); i >= 0 {
				t.Fatalf("unsubstituted token in %q — the only token this block defines is ${HOME}", want)
			}
			if got := normalizeCwd(c.In); got != want {
				t.Fatalf("normalizeCwd(%q) = %q, want %q\n  why: %s", c.In, got, want, c.Why)
			}
		})
	}
	// The block's whole reason for existing. Without a '~' vector this test is
	// satisfied by the tilde-expanding version that shipped.
	if !sawTilde {
		t.Fatal("no '~' vector RAN in the spawnCwds block — BINDING DECISION 1 is unpinned on this seam again")
	}
	if err := tally.RequireEvery("the spawnCwds block", spawnCwdFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

// The other half of BINDING DECISION 1 on this seam: expandTilde still exists
// for the PROFILE's configDir, which is host config rather than a caller's path,
// and it must not creep back into anything that touches caller input.
func TestNoCallerCwdPathExpandsATilde(t *testing.T) {
	src, err := os.ReadFile("profiles.go")
	if err != nil {
		t.Fatal(err)
	}
	// CRLF-normalized: this guard delimits the function TEXTUALLY ("\n}\n"), and
	// GitHub's Windows runners check the repo out with CRLF, where that needle
	// never matches — the guard then failed with "could not delimit normalizeCwd"
	// rather than checking anything. How the checkout spells a newline is not
	// what this test is about.
	body := strings.ReplaceAll(string(src), "\r\n", "\n")
	i := strings.Index(body, "func normalizeCwd(")
	if i < 0 {
		t.Fatal("normalizeCwd is gone — this guard has stopped guarding anything")
	}
	end := strings.Index(body[i:], "\n}\n")
	if end < 0 {
		t.Fatal("could not delimit normalizeCwd")
	}
	if strings.Contains(body[i:i+end], "expandTilde") {
		t.Error("normalizeCwd calls expandTilde again — a caller-supplied cwd must not be tilde-expanded (BINDING DECISION 1), and the desktop twin does not")
	}
}
