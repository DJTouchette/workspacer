//go:build !windows

package taskartifacts

import (
	"fmt"
	"os"
	"path/filepath"
)

func MakePrivateDirectory(dir string) error {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	return VerifyPrivateDirectory(dir)
}

func VerifyPrivateDirectory(dir string) error {
	info, err := os.Lstat(dir)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 || info.Mode().Perm()&0077 != 0 {
		return fmt.Errorf("task storage must be a private real directory")
	}
	return nil
}

func CommitFile(from, to string) error {
	if err := os.Rename(from, to); err != nil {
		return err
	}
	d, err := os.Open(filepath.Dir(to))
	if err != nil {
		return err
	}
	defer d.Close()
	return d.Sync()
}

func VerifyPrivateTree(dir string) error { return VerifyPrivateDirectory(dir) }

func SamePath(a, b string) bool            { return filepath.Clean(a) == filepath.Clean(b) }
func SameSourceDirectory(a, b string) bool { return SamePath(a, b) }
