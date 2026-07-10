//go:build !windows

package supervisor

import (
	"os"
	"syscall"
)

// terminate asks a sidecar to shut down gracefully (SIGTERM); os/exec's
// WaitDelay escalates to Kill if it lingers.
func terminate(p *os.Process) error { return p.Signal(syscall.SIGTERM) }
