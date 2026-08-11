package plugin

import (
	"runtime"
	"strings"
	"testing"
)

// absTestPath builds a path that is ABSOLUTE on the platform running the test.
//
// The obvious spelling is wrong on Windows: absTestPath("plugins", "acme")
// yields `\plugins\acme`, which is DRIVE-RELATIVE — it resolves against whatever
// drive the process happens to be on, so it is not absolute and every guard here
// correctly drops it. A case built that way then asserts that expandScope
// returns a path the guard could never accept, and fails on Windows for a reason
// that has nothing to do with what it is testing.
//
// Nothing on disk is touched: these are synthetic roots for string-level
// containment cases, which is why a fixed drive is fine.
// It deliberately does NOT use filepath.Join: Join Cleans, and several cases
// here are ABOUT the ".." segments Clean would collapse.
func absTestPath(parts ...string) string {
	root, sep := "/", "/"
	if runtime.GOOS == "windows" {
		root, sep = `C:\`, `\`
	}
	return root + strings.Join(parts, sep)
}

// used keeps the helper from tripping the unused-symbol check when a future edit
// removes its last caller; it is a test seam, not dead code.
var _ = func(t *testing.T) { _ = absTestPath("x") }
