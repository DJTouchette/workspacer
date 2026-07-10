//go:build !windows

package parentwatch

import (
	"syscall"
	"time"
)

// How often the parent-pid safety net checks whether the launcher is still alive.
const pollInterval = time.Second

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

// watchParent polls the launcher pid once a second until it disappears, then
// calls fire. (Unix parents that die are re-parented predictably and PIDs
// recycle slowly, so a poll is sufficient here; Windows pins a handle instead
// — see parentwatch_windows.go.)
func watchParent(pid int, fire func(reason string)) {
	go func() {
		for {
			time.Sleep(pollInterval)
			if !parentAlive(pid) {
				fire("parent process gone")
				return
			}
		}
	}()
}
