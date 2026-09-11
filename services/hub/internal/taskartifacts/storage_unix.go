//go:build !windows

package taskartifacts

import (
	"golang.org/x/sys/unix"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
)

func freeStorageBytes(dir string) (uint64, error) {
	var st unix.Statfs_t
	err := unix.Statfs(dir, &st)
	return uint64(st.Bavail) * uint64(st.Bsize), err
}
func confineGitChild() error {
	if err := syscall.Setpgid(0, 0); err != nil {
		return err
	}
	return unix.Setrlimit(unix.RLIMIT_FSIZE, &unix.Rlimit{Cur: GitPackLimit, Max: GitPackLimit})
}
func stopGitChild() { _ = syscall.Kill(-os.Getpid(), syscall.SIGKILL); os.Exit(1) }
func cancelStorageCommand(cmd *exec.Cmd) error {
	_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
	return cmd.Process.Kill()
}

func lockStorage(root string) (func(), error) {
	f, err := os.OpenFile(filepath.Join(root, "storage.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	if err := unix.Flock(int(f.Fd()), unix.LOCK_EX|unix.LOCK_NB); err != nil {
		f.Close()
		return nil, err
	}
	return func() { _ = f.Close() }, nil
}
