package main

// Optional supervision of the brain capability provider (cmd/brain). When
// enabled, the hub spawns the brain as a child and keeps it alive, so the bus
// always has a provider for the agent/config/library/… capabilities WITHOUT the
// desktop app — the headless "single source of truth". The hub still only
// routes; the brain (a separate process) is what executes, exactly like any
// other provider.
//
//	--brain-scope off      (default) don't spawn a brain
//	--brain-scope full     spawn a brain that provides the whole surface (headless)
//	--brain-scope catalog  spawn a brain that provides only the file-backed subset
//	                       (run this when the desktop app owns the live agent caps)

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
)

func brainExeName() string {
	if runtime.GOOS == "windows" {
		return "brain.exe"
	}
	return "brain"
}

// resolveBrainBin finds the brain binary: an explicit flag wins; otherwise a
// sibling of the hub executable (where `make build-hub` / packaging put it);
// otherwise PATH. Returns "" when nothing is found.
func resolveBrainBin(flagVal string) string {
	if flagVal != "" {
		return flagVal
	}
	if exe, err := os.Executable(); err == nil {
		cand := filepath.Join(filepath.Dir(exe), brainExeName())
		if st, err := os.Stat(cand); err == nil && !st.IsDir() {
			return cand
		}
	}
	if p, err := exec.LookPath("brain"); err == nil {
		return p
	}
	return ""
}

// brainArgs builds the brain's argv from the hub's own settings so the two agree
// on bus URL, auth, claudemon, and scope.
func brainArgs(addr, claudemonURL, scope string) []string {
	return []string{
		"--hub", "ws://" + addr + "/bus",
		"--claudemon", claudemonURL,
		"--scope", scope,
	}
}
