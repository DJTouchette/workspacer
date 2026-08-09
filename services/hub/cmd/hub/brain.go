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
	"net"
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
		"--hub", "ws://" + busDialAddr(addr) + "/bus",
		"--claudemon", claudemonURL,
		"--scope", scope,
	}
}

// busDialAddr turns the hub's BIND address into one its own child can dial.
//
// A bind address may name a wildcard — "0.0.0.0:7895", "[::]:7895", ":7895" —
// which is not an address at all: it means "every interface". Handing that
// straight to a dialer used to work by accident (connect(0.0.0.0) lands on
// loopback), and the URL doubles as the child's `Host` header, which is where it
// stopped working: requireHost refuses a Host that is neither loopback nor the
// address the socket landed on, so `Host: 0.0.0.0:7895` is a 403.
//
// The failure was silent and total. The brain's output is discarded (its Spec
// sets no LogLines), so it reconnected into that 403 for ever with nothing in
// any log, while every capability it is the sole provider for — config.get /
// config.save, library.*, layouts.*, sessions.list/load/save/delete,
// claude.profiles.*, claude.listModels, fs.* — answered "no provider" on the
// bus. The desktop renderer runs in bus mode by default, so that reached the
// user as settings which silently refuse to persist (a pinned widget board
// vanishing on the next read was how it was found), and only when remote
// sharing was on, because only then is the bind a wildcard.
//
// The child is always on this machine, so loopback is both dialable and the one
// Host the pin accepts unconditionally.
func busDialAddr(addr string) string {
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		// Not host:port (a bare name, or already malformed). Nothing safe to
		// rewrite — hand it back and let the dial report the real problem.
		return addr
	}
	switch host {
	case "", "0.0.0.0":
		host = "127.0.0.1"
	case "::":
		host = "::1"
	}
	return net.JoinHostPort(host, port)
}
