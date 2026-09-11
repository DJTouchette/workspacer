package taskartifacts

import (
	"github.com/djtouchette/workspacer-hub/internal/jobobject"
	"golang.org/x/sys/windows"
	"os"
	"os/exec"
)

func freeStorageBytes(dir string) (uint64, error) {
	p, err := windows.UTF16PtrFromString(dir)
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
