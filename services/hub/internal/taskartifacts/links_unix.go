//go:build !windows

package taskartifacts

import (
	"os"
	"syscall"
)

func singleLink(_ *os.File, info os.FileInfo) bool {
	st, ok := info.Sys().(*syscall.Stat_t)
	return ok && st.Nlink == 1
}
