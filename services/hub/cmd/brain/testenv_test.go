package main

import (
	"runtime"
	"strings"
	"testing"
)

// Redirecting the process's notion of "home" and "config dir" in a test, on
// EVERY platform.
//
// configDir() reads APPDATA on Windows and XDG_CONFIG_HOME elsewhere; homeDir()
// goes through os.UserHomeDir(), which reads USERPROFILE on Windows and HOME
// elsewhere. A test that sets only the POSIX half therefore redirects NOTHING on
// Windows: it runs against the machine's real config dir and real home. That is
// how the whole cmd/brain config suite failed the first time this package ever
// compiled on a Windows runner — and, more quietly, how a test that happened to
// pass there was asserting against the runner's own profile rather than its
// fixture.
//
// Setting both halves is a no-op on POSIX (nothing reads APPDATA or USERPROFILE
// there) and is what makes these tests mean the same thing on both platforms.

// tempConfigHome points configDir() at a fresh temp dir and returns it.
func tempConfigHome(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	setConfigHome(t, dir)
	return dir
}

// setConfigHome points configDir() at an EXISTING dir the caller owns.
func setConfigHome(t *testing.T, dir string) {
	t.Helper()
	t.Setenv("XDG_CONFIG_HOME", dir)
	t.Setenv("APPDATA", dir)
}

// tempHome points homeDir()/os.UserHomeDir() at a fresh temp dir and returns it.
func tempHome(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	setHome(t, dir)
	return dir
}

// setHome points homeDir()/os.UserHomeDir() at an EXISTING dir the caller owns.
func setHome(t *testing.T, dir string) {
	t.Helper()
	t.Setenv("HOME", dir)
	t.Setenv("USERPROFILE", dir)
}

// absTestPath builds a path that is ABSOLUTE on the platform running the test.
//
// A bare "/proj" is not absolute on Windows — it is DRIVE-relative, resolving
// against whatever drive the process is on — so a guard or a path join that is
// correct will refuse or mangle it, and the case fails for a reason unrelated to
// what it tests. Nothing on disk is touched; these are synthetic roots.
//
// Deliberately not filepath.Join: Join Cleans, and some callers pass ".."
// segments that the case is about.
func absTestPath(parts ...string) string {
	root, sep := "/", "/"
	if runtime.GOOS == "windows" {
		root, sep = `C:\`, `\`
	}
	return root + strings.Join(parts, sep)
}
