//go:build windows

package jobobject

import (
	"unsafe"

	"golang.org/x/sys/windows"
)

// Confine creates an anonymous kill-on-close job object and assigns the
// CURRENT process to it. Children (and their children) created afterwards are
// placed in the same job automatically, so the whole tree dies with us. The
// job handle is deliberately never closed — the process's own death closes it,
// which is exactly the trigger.
//
// Nested jobs are supported since Windows 8, so this also works when a parent
// (an IDE, a service wrapper) already has us in a job of its own. Errors are
// returned for logging but should be treated as non-fatal: the parentwatch
// self-exit still applies; this is the belt to its braces.
func Confine() error {
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return err
	}
	info := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{
		BasicLimitInformation: windows.JOBOBJECT_BASIC_LIMIT_INFORMATION{
			LimitFlags: windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
		},
	}
	if _, err := windows.SetInformationJobObject(
		job,
		windows.JobObjectExtendedLimitInformation,
		uintptr(unsafe.Pointer(&info)),
		uint32(unsafe.Sizeof(info)),
	); err != nil {
		_ = windows.CloseHandle(job)
		return err
	}
	if err := windows.AssignProcessToJobObject(job, windows.CurrentProcess()); err != nil {
		_ = windows.CloseHandle(job)
		return err
	}
	// Intentionally keep `job` open for our whole lifetime.
	return nil
}
