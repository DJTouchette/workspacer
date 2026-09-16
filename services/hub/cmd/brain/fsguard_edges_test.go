package main

// The branches of fsguard.go the corpus structurally cannot reach: the two
// fail-closed fallbacks (an unresolvable home dir, an unverifiable target) and
// the cwd cache's expiry. Each of these could be flipped to fail OPEN with the
// entire corpus and the whole Go suite green, because a fixture case supplies a
// well-formed sandbox by construction and can never withhold one.

import (
	"path/filepath"
	"testing"
)

// browseRoots is the picker's allow-list and its home-dir lookup has exactly one
// error branch — the ONLY fail-closed fallback fs.listDir has. Nothing made
// os.UserHomeDir fail, so `return r.workspaceRoots(ctx)` could become
// `home = "/"` and fs.listDir would enumerate /etc for a bus client with no live
// agents, with the suite green.// pathIsSecret's `return true` on an unverifiable target. The corpus asserts the
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
// project staying readable and writable — with the suite green.// containsPath's empty-root arm. canonicalRoot discards a root it cannot
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
