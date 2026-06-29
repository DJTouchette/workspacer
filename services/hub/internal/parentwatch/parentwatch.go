// Package parentwatch lets a daemon self-exit when the process that launched it
// dies, so a force-killed or crashed parent never leaves orphaned children
// holding ports.
//
// Detection is via stdin EOF: the launcher (the desktop app, or the hub
// supervisor for brain/sidecars) hands the child a stdin pipe and holds the
// write end open for its whole life. When the launcher dies — even on a
// force-kill the OS can't notify us about — the kernel closes the pipe and our
// read hits EOF. Cross-platform (Win/Mac/Linux), no polling, no native APIs.
package parentwatch

import (
	"io"
	"log"
	"os"
)

// Watch starts a goroutine that calls onParentExit once stdin reaches EOF,
// which happens when the launcher process dies. It is gated on
// WORKSPACER_PARENT_PID (set by the launcher) so a manual run from a terminal —
// where stdin is a TTY or /dev/null — never triggers a spurious shutdown. When
// the env var is unset, Watch is a no-op.
func Watch(onParentExit func()) {
	if os.Getenv("WORKSPACER_PARENT_PID") == "" {
		return
	}
	go func() {
		// Blocks until the parent closes its end of the pipe. Any bytes the
		// parent happens to write are discarded; only EOF matters.
		_, _ = io.Copy(io.Discard, os.Stdin)
		log.Println("parentwatch: parent process exited (stdin closed); shutting down")
		onParentExit()
	}()
}
