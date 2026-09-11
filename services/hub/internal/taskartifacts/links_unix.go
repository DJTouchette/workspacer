//go:build !windows

package taskartifacts

import (
	"fmt"
	"os"
	"syscall"
)

func DirectoryIdentity(dir string) (string, error) {
	info, err := os.Lstat(dir)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return "", fmt.Errorf("allocation directory unavailable")
	}
	st, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		return "", fmt.Errorf("allocation identity unavailable")
	}
	return fmt.Sprintf("unix:%x:%x", st.Dev, st.Ino), nil
}

func singleLink(_ *os.File, info os.FileInfo) bool {
	st, ok := info.Sys().(*syscall.Stat_t)
	return ok && st.Nlink == 1
}
