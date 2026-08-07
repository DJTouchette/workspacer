package bus

import (
	"os"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// ---------------------------------------------------------------------------
// The bus's copy of the host-gate floor. See cmd/brain/hostgate_test.go for the
// argument; the short version is that a Go test which t.Skipf's itself whole
// leaves the package `ok`, so the two symlink-shaped guards in this package —
// the ONLY end-to-end proof that conn.authorize refuses a traversal, and the
// only input that reaches canonicalize's ELOOP arm — could both stop running
// with nothing to show for it.
//
// TWIN: cmd/brain/hostgate_test.go. Same gate, same lever
// (WKS_TEST_NO_SYMLINKS=1), and the meta-guard in cmd/brain checks that both
// packages still have one.
// ---------------------------------------------------------------------------

var symlinkGate = sweepguard.Gate("internal/bus's symlink-gated tests", 2)

func TestMain(m *testing.M) {
	os.Exit(sweepguard.RunGates(m))
}

func gatedTestName(t *testing.T) string {
	t.Helper()
	return strings.SplitN(t.Name(), "/", 2)[0]
}

func hostFeatureDisabled(feature string) bool {
	v := strings.TrimSpace(os.Getenv("WKS_TEST_NO_" + feature))
	return v != "" && v != "0" && !strings.EqualFold(v, "false")
}

// gateSymlink is os.Symlink for a test with nothing left to assert without one.
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

// gateSymlinkOptional is for the softer shape: a test that keeps running without
// the symlink but silently drops the ONE input that reaches an arm nothing else
// reaches (canonicalize's ELOOP). It does not skip — it records — because the
// loss is a coverage loss and not a failed setup, and a coverage loss that is
// invisible is the defect this file exists to make impossible.
func gateSymlinkOptional(t *testing.T, target, link string) bool {
	t.Helper()
	name := gatedTestName(t)
	if hostFeatureDisabled("SYMLINKS") {
		symlinkGate.Skip(name, "needsSymlinks (WKS_TEST_NO_SYMLINKS)")
		return false
	}
	if err := os.Symlink(target, link); err != nil {
		symlinkGate.Skip(name, "needsSymlinks: "+err.Error())
		return false
	}
	gateCleared(t, symlinkGate, name)
	return true
}

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
