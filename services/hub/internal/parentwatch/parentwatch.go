// Package parentwatch lets a daemon self-exit when the process that launched it
// dies, so a force-killed or crashed parent never leaves orphaned children
// holding ports.
//
// Two independent triggers race, and whichever fires first wins:
//
//  1. stdin EOF — the launcher (the desktop app, or the hub supervisor for
//     brain/sidecars) hands the child a stdin pipe and holds the write end open
//     for its whole life. When the launcher dies — even on a force-kill the OS
//     can't notify us about — the kernel closes the pipe and our read hits EOF.
//     Fastest path when it works.
//  2. parent-pid poll — the safety net for Windows, where libuv marks the stdio
//     pipe handles inheritable, so a sibling daemon inherits a duplicate of our
//     stdin write handle. That duplicate keeps the pipe open after the launcher
//     dies, so EOF never arrives and the daemons hold their ports until killed
//     by hand. Polling WORKSPACER_PARENT_PID for the launcher's death doesn't
//     depend on the pipe, so it frees the ports regardless.
package parentwatch

import (
	"io"
	"log"
	"os"
	"strconv"
	"sync"
)

// Watch starts background goroutines that call onParentExit once the launcher
// process dies — detected via stdin EOF and/or a parent-pid poll. It is gated on
// WORKSPACER_PARENT_PID (set by the launcher) so a manual run from a terminal —
// where stdin is a TTY or /dev/null and there is no launcher to watch — never
// triggers a spurious shutdown. When the env var is unset, Watch is a no-op.
func Watch(onParentExit func()) {
	pidStr := os.Getenv("WORKSPACER_PARENT_PID")
	if pidStr == "" {
		return
	}

	var once sync.Once
	fire := func(reason string) {
		once.Do(func() {
			log.Printf("parentwatch: %s; shutting down", reason)
			onParentExit()
		})
	}

	// Trigger 1: stdin EOF. Any bytes the parent happens to write are discarded;
	// only EOF matters.
	go func() {
		_, _ = io.Copy(io.Discard, os.Stdin)
		fire("parent process exited (stdin closed)")
	}()

	// Trigger 2: watch the launcher pid (platform-specific — a pinned process
	// handle on Windows, a liveness poll elsewhere). Skipped if the pid didn't
	// parse, in which case we rely on the EOF trigger alone.
	if pid, err := strconv.Atoi(pidStr); err == nil && pid > 0 {
		watchParent(pid, fire)
	}
}
