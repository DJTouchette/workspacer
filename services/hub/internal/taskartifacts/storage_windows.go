package taskartifacts

import (
	"github.com/djtouchette/workspacer-hub/internal/jobobject"
	"golang.org/x/sys/windows"
	"os"
	"os/exec"
	"path/filepath"
)

func freeStorageBytes(dir string) (uint64, error) {
	p, err := windows.UTF16PtrFromString(nativePath(dir))
	if err != nil {
		return 0, err
	}
	var free, total, totalFree uint64
	err = windows.GetDiskFreeSpaceEx(p, &free, &total, &totalFree)
	return free, err
}
func confineGitChild() error                   { return jobobject.Confine() }
func stopGitChild()                            { os.Exit(1) } // Kill-on-close job terminates every Git child.
func cancelStorageCommand(cmd *exec.Cmd) error { return cmd.Process.Kill() }

func lockStorage(root string) (func(), error) {
	f, err := os.OpenFile(filepath.Join(root, "storage.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	var overlap windows.Overlapped
	if err := windows.LockFileEx(windows.Handle(f.Fd()), windows.LOCKFILE_EXCLUSIVE_LOCK|windows.LOCKFILE_FAIL_IMMEDIATELY, 0, 1, 0, &overlap); err != nil {
		f.Close()
		return nil, err
	}
	return func() { _ = f.Close() }, nil
}
