//go:build windows

package parentwatch

import "syscall"

// SYNCHRONIZE is the minimal access right needed to wait on a process handle.
// Not exported by the std syscall package on Windows, so we define it locally.
const _SYNCHRONIZE = 0x0010_0000

// parentAlive reports whether process pid is still running. It opens a handle to
// the pid and checks whether the process object has become signaled — which on
// Windows means it has exited. Failure to open the pid means it's already gone;
// any wait error is treated as "alive" so we never shut down while the launcher
// is up.
func parentAlive(pid int) bool {
	h, err := syscall.OpenProcess(_SYNCHRONIZE, false, uint32(pid))
	if err != nil {
		return false // can't open the pid → it's gone
	}
	defer syscall.CloseHandle(h)
	// Zero timeout: return immediately with the current signaled state.
	event, err := syscall.WaitForSingleObject(h, 0)
	if err != nil {
		return true // uncertain — err on the side of staying up
	}
	// WAIT_OBJECT_0 (0) means the process object is signaled == it has exited.
	return event != syscall.WAIT_OBJECT_0
}
