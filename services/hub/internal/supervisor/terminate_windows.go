//go:build windows

package supervisor

import "os"

// terminate kills the sidecar outright: Windows has no SIGTERM delivery —
// Process.Signal(SIGTERM) just errors, which used to leave the sidecar running
// until WaitDelay's 5s force-kill (× every sidecar, serially, on shutdown).
// Sidecars are supervised, stateless-by-design processes; an immediate
// TerminateProcess is the correct Windows semantics. The hub's kill-on-close
// job object backstops any grandchildren.
func terminate(p *os.Process) error { return p.Kill() }
