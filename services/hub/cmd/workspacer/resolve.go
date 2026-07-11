package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
)

// exeName appends the platform executable suffix, so the same resolution code
// finds "claudemon.exe" on Windows and "claudemon" elsewhere.
func exeName(name string) string {
	if runtime.GOOS == "windows" {
		return name + ".exe"
	}
	return name
}

// resolveBin finds a sibling daemon binary (claudemon, hub, brain). The order
// mirrors how the hub itself locates the brain (cmd/hub resolveBrainBin), so
// a packaged install — where every binary sits in one directory — works with
// zero flags, while developers can still point at a build elsewhere:
//
//  1. an explicit --<name>-bin flag wins;
//  2. a sibling of this executable (where `make build-cli` / packaging put it);
//  3. PATH.
//
// Returns "" when nothing is found — callers decide whether that's fatal.
func resolveBin(name, override, siblingDir string) string {
	if override != "" {
		return override
	}
	if siblingDir != "" {
		cand := filepath.Join(siblingDir, exeName(name))
		if st, err := os.Stat(cand); err == nil && !st.IsDir() {
			return cand
		}
	}
	if p, err := exec.LookPath(name); err == nil {
		return p
	}
	return ""
}

// selfDir is the directory holding this executable, symlinks resolved — so an
// install-cli symlink on PATH still finds its true siblings next to the real
// binary, not next to the link. Empty on (unlikely) failure, which just
// degrades resolution to PATH-only.
func selfDir() string {
	exe, err := os.Executable()
	if err != nil {
		return ""
	}
	if real, err := filepath.EvalSymlinks(exe); err == nil {
		exe = real
	}
	return filepath.Dir(exe)
}
