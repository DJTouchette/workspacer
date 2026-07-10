//go:build windows

package parentwatch

import "syscall"

// SYNCHRONIZE is the minimal access right needed to wait on a process handle.
// Not exported by the std syscall package on Windows, so we define it locally.
const _SYNCHRONIZE = 0x0010_0000

const _INFINITE = 0xFFFF_FFFF

// watchParent blocks-in-background until the launcher process exits, then
// calls fire.
//
// It opens a handle to the pid ONCE, up front, and waits on that handle. This
// matters: Windows recycles PIDs aggressively, so re-opening the pid on every
// poll (the old implementation) races PID reuse — if another process claimed
// the launcher's pid between polls, the watcher believed the launcher was
// still alive forever and the daemon held its ports until killed by hand. A
// pinned handle references the original process object, which stays waitable
// (signaled once it exits) no matter who inherits the pid number.
func watchParent(pid int, fire func(reason string)) {
	h, err := syscall.OpenProcess(_SYNCHRONIZE, false, uint32(pid))
	if err != nil {
		// Can't open the pid → the launcher is already gone.
		fire("parent process already gone (open failed)")
		return
	}
	go func() {
		defer syscall.CloseHandle(h)
		// Blocks until the process object is signaled (= the launcher exited).
		_, _ = syscall.WaitForSingleObject(h, _INFINITE)
		fire("parent process exited (handle signaled)")
	}()
}
