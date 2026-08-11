package main

import "testing"

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
