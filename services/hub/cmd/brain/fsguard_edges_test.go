package main

// The branches of fsguard.go the corpus structurally cannot reach: the two
// fail-closed fallbacks (an unresolvable home dir, an unverifiable target) and
// the cwd cache's expiry. Each of these could be flipped to fail OPEN with the
// entire corpus and the whole Go suite green, because a fixture case supplies a
// well-formed sandbox by construction and can never withhold one.

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// browseRoots is the picker's allow-list and its home-dir lookup has exactly one
// error branch — the ONLY fail-closed fallback fs.listDir has. Nothing made
// os.UserHomeDir fail, so `return r.workspaceRoots(ctx)` could become
// `home = "/"` and fs.listDir would enumerate /etc for a bus client with no live
// agents, with the suite green.
func TestBrowseRootsFallsBackToTheWorkspaceWhenThereIsNoHome(t *testing.T) {
	sandbox := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	// os.UserHomeDir reads $HOME on unix and %USERPROFILE% on Windows; empty is
	// the systemd/launchd/container case it is meant to survive.
	t.Setenv("HOME", "")
	t.Setenv("USERPROFILE", "")
	if _, err := os.UserHomeDir(); err == nil {
		t.Skip("this platform resolves a home directory without the environment")
	}

	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)
	reg := newRegistry(nil)
	ctx := context.Background()

	browse := reg.browseRoots(ctx)
	work := reg.workspaceRoots(ctx)
	if strings.Join(browse, "\x00") != strings.Join(work, "\x00") {
		t.Fatalf("with no resolvable home the picker must degrade to the workspace roots\n  browse: %v\n  work:   %v", browse, work)
	}
	for _, r := range browse {
		if r == string(filepath.Separator) {
			t.Fatalf("browseRoots handed out the filesystem root: %v", browse)
		}
	}

	// And behaviourally: the picker must not list a system directory.
	for _, target := range []string{string(filepath.Separator), filepath.Join(string(filepath.Separator), "etc")} {
		if _, err := reg.handle(ctx, "fs.listDir", json.RawMessage(`{"path":`+jsonStr(target)+`}`)); err == nil {
			t.Errorf("fs.listDir listed %q with no home directory and no live agents", target)
		}
	}
}

// pathIsSecret's `return true` on an unverifiable target. The corpus asserts the
// DECOMPOSITION `within && !secret == allowed`, and pathWithinRoots already
// returns false for exactly the targets that make pathIsSecret unverifiable — so
// the conjunction short-circuits and this branch was never observed. Flipping it
// to `return false` passed every case.
func TestPathIsSecretDeniesWhatItCannotVerify(t *testing.T) {
	sandbox := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))

	for _, target := range []string{
		"",                 // empty
		"   ",              // whitespace only
		"notes.txt",        // relative
		"./notes.txt",      // relative with a leading '.'
		"~/.ssh/id_rsa",    // BINDING DECISION 1: '~' is not expanded, so this is relative
		"~",                // and a bare tilde is too
		"../../etc/passwd", // relative traversal
	} {
		if !pathIsSecret(target) {
			t.Errorf("pathIsSecret(%q) = false; an unverifiable target must be denied, same posture as pathWithinRoots", target)
		}
		if pathWithinRoots([]string{sandbox}, target) {
			t.Errorf("pathWithinRoots(%q) = true; the sibling predicate must refuse it too", target)
		}
	}

	// The floor: a verifiable, ordinary path is NOT secret, or the assertion
	// above is satisfied by a predicate that denies everything.
	ordinary := filepath.Join(sandbox, "notes.txt")
	if pathIsSecret(ordinary) {
		t.Errorf("pathIsSecret(%q) = true; the gate must bite only inside the config dir and on the credential basenames", ordinary)
	}
}

// The cwd cache is the ONLY mechanism that revokes a root: an agent's cwd is an
// allowed root while it lives, and it leaves the allow-list when the cache
// expires and the re-read finds no such session. Nothing advanced time past the
// TTL, so the constant could be raised to an hour — a stopped agent's whole
// project staying readable and writable — with the suite green.
func TestStoppedAgentCwdLeavesTheAllowList(t *testing.T) {
	if cwdCacheTTL > cwdCacheTTLCeiling {
		t.Fatalf("cwdCacheTTL is %v, above the %v ceiling: this cache is the only thing that ever revokes a root", cwdCacheTTL, cwdCacheTTLCeiling)
	}
	// Shrink it so the expiry itself is asserted in milliseconds rather than
	// seconds; the shipped value is pinned by the ceiling above.
	restore := cwdCacheTTL
	cwdCacheTTL = 20 * time.Millisecond
	t.Cleanup(func() { cwdCacheTTL = restore })

	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	cwd := filepath.Join(sandbox, "proj")
	if err := os.MkdirAll(cwd, 0o755); err != nil {
		t.Fatal(err)
	}
	notes := filepath.Join(cwd, "notes.txt")
	if err := os.WriteFile(notes, []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}

	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)
	reg := registryWithCwd(t, cwd)
	ctx := context.Background()
	read := json.RawMessage(`{"path":` + jsonStr(notes) + `}`)
	if _, err := reg.handle(ctx, "fs.read", read); err != nil {
		t.Fatalf("a file in a LIVE agent's cwd must be readable: %v", err)
	}

	// The agent stops: the session store empties, but the cache still holds the
	// root until the TTL passes.
	reg.store = newSessionStore()
	deadline := time.Now().Add(2 * time.Second)
	for {
		if _, err := reg.handle(ctx, "fs.read", read); err != nil {
			return // revoked
		}
		if time.Now().After(deadline) {
			t.Fatalf("fs.read of a stopped agent's file was still allowed 2s after the session disappeared (TTL %v)", cwdCacheTTL)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// containsPath's empty-root arm. canonicalRoot discards a root it cannot
// resolve, so "" should never reach here — but the LAST LINE OF DEFENCE must not
// itself be the widest possible grant, and without the explicit test neither
// branch below sees a trailing separator and the comparison falls through to
// HasPrefix(target, "/"), which is true for every absolute path on the system.
// Twin: the bus's TestCanonRootsDiscardsWhatItCannotResolve.
func TestAnEmptyRootContainsNothing(t *testing.T) {
	for _, target := range []string{
		filepath.FromSlash("/etc/passwd"),
		filepath.FromSlash("/root/.ssh/id_rsa"),
		string(filepath.Separator),
		filepath.Join(t.TempDir(), "notes.txt"),
	} {
		if containsPath("", target) {
			t.Errorf(`containsPath("", %q) = true — the empty string is behaving as a wildcard root`, target)
		}
		if pathWithinRootsCanonical([]string{""}, target) {
			t.Errorf(`pathWithinRootsCanonical([""], %q) = true`, target)
		}
	}
	// The floor: the FILESYSTEM root really does contain everything (BINDING
	// DECISION 3), so "deny the empty string" must not be "deny short roots".
	vol := string(filepath.Separator)
	if !containsPath(vol, filepath.FromSlash("/etc/passwd")) {
		t.Errorf("containsPath(%q, /etc/passwd) = false; a volume root contains everything below it", vol)
	}
}
