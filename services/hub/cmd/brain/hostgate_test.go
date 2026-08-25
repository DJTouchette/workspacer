package main

import (
	"os"
	"os/exec"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// ---------------------------------------------------------------------------
// The floor for this package's HOST-GATED tests.
//
// A fixture sweep proves it ran with a sweepguard.Tally. The tests below are the
// other kind: hand-written, one scenario each, and every one of them needs a
// privilege the host may not have — symlink creation, a `git` binary. Their
// shape was `if err := os.Symlink(...); err != nil { t.Skipf(...) }`, which is a
// SKIP, and a skipped Go test does not colour the package: `go test ./...`
// prints `ok` all the same.
//
// Simulate a host without symlink privilege (WKS_TEST_NO_SYMLINKS=1) against the
// tree before this file existed and cmd/brain still printed ok while 19 tests —
// every symlink-escape guard in the brain, which is most of what the confinement
// work produced — asserted nothing. The desktop side has had the twin of this
// (gatedIt + itRanEveryGatedTest) since the sessions-store finding; the Go side
// had nothing, and that asymmetry is what this file closes.
//
// The gates are registered with the process, and TestMain re-checks them after
// the suite: a floor that lived only in a test function can be filtered out,
// renamed away, or never written — which is exactly how this hole was dug.
// ---------------------------------------------------------------------------

// symlinkGate counts the tests that cannot assert ANYTHING without creating a
// symlink. The number is the size of the group; adding a gated test without
// raising it fails, so the count cannot go stale in either direction.
var symlinkGate = sweepguard.Gate("cmd/brain's symlink-gated tests", 27)

// gitGate counts the tests that shell out to a real `git`.
var gitGate = sweepguard.Gate("cmd/brain's git-gated tests", 12)

// sandboxConfigHome points the whole test BINARY's configDir() at a temp dir.
//
// configDir() reads XDG_CONFIG_HOME (APPDATA on Windows) and every config-backed
// service in this package resolves through it — configService, the read-time
// keybinding migrations, the auth-token store. A test that constructs any of
// them without calling tempConfigHome/setConfigHome therefore runs against the
// DEVELOPER'S OWN ~/.config/workspacer, and dozens do (newRegistry alone builds
// a configService, and 47 tests call it).
//
// Reading it was already wrong. Writing it was destructive: on 2026-08-23 a
// single test that built a defaults-shaped map and handed it to
// migrateKeybindings — which writes through to disk — reset the machine's real
// config.yaml to factory settings on every `go test ./...`, repeatedly, faster
// than it could be restored by hand. writeConfigYAML now refuses that specific
// write (refuseWipeWithDefaults), but a per-call refusal is the wrong place to
// rely on: the durable property is that this binary cannot address the real
// config dir at all.
//
// Set before m.Run, so it covers every test including those with no redirect of
// their own. Tests that DO redirect (t.Setenv via tempConfigHome/setConfigHome)
// override it for their duration and restore it afterwards, unchanged.
//
// HOME is deliberately left alone: configDir() only falls back to it when
// XDG_CONFIG_HOME is empty, which it no longer is, and tests that need a
// sandboxed home have tempHome for exactly that.
func sandboxConfigHome() (cleanup func()) {
	dir, err := os.MkdirTemp("", "wks-brain-confighome-")
	if err != nil {
		// Nothing to sandbox with. Fail loudly rather than silently running the
		// suite against the developer's real config dir — that is the outcome
		// this exists to prevent.
		panic("cmd/brain: cannot create a sandbox config home: " + err.Error())
	}
	for _, k := range []string{"XDG_CONFIG_HOME", "APPDATA"} {
		if err := os.Setenv(k, dir); err != nil {
			panic("cmd/brain: cannot redirect " + k + ": " + err.Error())
		}
	}
	return func() { _ = os.RemoveAll(dir) }
}

func TestMain(m *testing.M) {
	cleanup := sandboxConfigHome()
	code := sweepguard.RunGates(m)
	cleanup()
	os.Exit(code)
}

// gatedTestName is the TOP-LEVEL test a gate counts. Subtests are not counted
// apart: a table-driven test that links once per row is one test, and keying on
// the full subtest name would make the declared count a function of the table's
// length.
func gatedTestName(t *testing.T) string {
	t.Helper()
	return strings.SplitN(t.Name(), "/", 2)[0]
}

// hostFeatureDisabled is the lever that makes this machinery falsifiable on a
// developer machine that HAS every privilege. WKS_TEST_NO_SYMLINKS=1 /
// WKS_TEST_NO_GIT=1 simulate the host that does not. Without it the only way to
// see the floor bite is to find such a host, which is precisely why the hole
// survived: the failure is invisible everywhere anyone looks.
func hostFeatureDisabled(feature string) bool {
	v := strings.TrimSpace(os.Getenv("WKS_TEST_NO_" + feature))
	return v != "" && v != "0" && !strings.EqualFold(v, "false")
}

// gateSymlink is os.Symlink for a test that has nothing left to assert without
// one. It records the outcome against symlinkGate and skips the test on failure,
// so the skip is counted rather than silent.
func gateSymlink(t *testing.T, target, link string) {
	t.Helper()
	name := gatedTestName(t)
	if hostFeatureDisabled("SYMLINKS") {
		symlinkGate.Skip(name, "needsSymlinks (WKS_TEST_NO_SYMLINKS)")
		t.Skip("needsSymlinks: WKS_TEST_NO_SYMLINKS simulates a host without symlink privilege")
	}
	if err := os.Symlink(target, link); err != nil {
		symlinkGate.Skip(name, "needsSymlinks: "+err.Error())
		t.Skipf("symlinks unavailable: %v", err)
	}
	gateCleared(t, symlinkGate, name)
}

// gateCleared records that `name` got past this gate, and arms the counter
// against the OTHER way a gated test asserts nothing: clearing the gate and then
// skipping for an unrelated reason further down (no git, a filename this
// filesystem will not hold). t.Skipped() is only knowable at cleanup, so the
// verdict is revised there. Only the top-level test arms it — a subtest that
// skips does not mean the test did nothing.
func gateCleared(t *testing.T, g *sweepguard.GateCounter, name string) {
	t.Helper()
	g.Ran(name)
	if t.Name() != name {
		return
	}
	t.Cleanup(func() {
		if t.Skipped() {
			g.Skip(name, "cleared the gate and then skipped for another reason")
		}
	})
}

// gateGit skips a test that needs a real git binary, counted the same way.
func gateGit(t *testing.T) {
	t.Helper()
	name := gatedTestName(t)
	if hostFeatureDisabled("GIT") {
		gitGate.Skip(name, "git disabled (WKS_TEST_NO_GIT)")
		t.Skip("WKS_TEST_NO_GIT simulates a host with no git binary")
	}
	if _, err := exec.LookPath("git"); err != nil {
		gitGate.Skip(name, "git not on PATH")
		t.Skip("git not on PATH")
	}
	gateCleared(t, gitGate, name)
}
