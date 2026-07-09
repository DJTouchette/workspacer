//go:build !windows

package parentwatch

import "syscall"

// parentAlive reports whether process pid is still running. Signal 0 delivers
// nothing; it just probes existence. EPERM means the process exists but we may
// not signal it (still alive); ESRCH (or anything else) means it's gone. On any
// ambiguity we err toward "alive" so we never shut down while the launcher is up.
func parentAlive(pid int) bool {
	err := syscall.Kill(pid, 0)
	if err == nil {
		return true
	}
	return err == syscall.EPERM
}
