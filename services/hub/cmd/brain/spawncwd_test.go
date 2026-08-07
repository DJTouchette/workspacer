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
)

type spawnCwdCase struct {
	In  string `json:"in"`
	Out string `json:"out"`
	Why string `json:"why"`
}

func loadSpawnCwdCases(t *testing.T) []spawnCwdCase {
	t.Helper()
	path := filepath.Join("..", "..", "..", "..", "contracts", "path-containment-cases.json")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Skipf("contract fixture not reachable from this checkout: %v", err)
	}
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

func TestSpawnCwdNormalizationContractCases(t *testing.T) {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		t.Skip("this process has no resolvable home directory")
	}
	cases := loadSpawnCwdCases(t)

	sawTilde := false
	for _, c := range cases {
		if strings.HasPrefix(c.In, "~") {
			sawTilde = true
		}
		t.Run(c.In, func(t *testing.T) {
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
		t.Fatal("no '~' vector in the spawnCwds block — BINDING DECISION 1 is unpinned on this seam again")
	}
}

// The other half of BINDING DECISION 1 on this seam: expandTilde still exists
// for the PROFILE's configDir, which is host config rather than a caller's path,
// and it must not creep back into anything that touches caller input.
func TestNoCallerCwdPathExpandsATilde(t *testing.T) {
	src, err := os.ReadFile("profiles.go")
	if err != nil {
		t.Fatal(err)
	}
	body := string(src)
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
