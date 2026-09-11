//go:build !windows

package taskartifacts

import (
	"fmt"
	"os"
)

func MakePrivateDirectory(dir string) error {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	info, err := os.Lstat(dir)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 || info.Mode().Perm()&0077 != 0 {
		return fmt.Errorf("task storage must be a private real directory")
	}
	return nil
}
