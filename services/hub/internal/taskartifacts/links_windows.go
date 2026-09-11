package taskartifacts

import (
	"fmt"
	"golang.org/x/sys/windows"
	"os"
)

func DirectoryIdentity(dir string) (string, error) {
	before, err := os.Lstat(dir)
	if err != nil || !before.IsDir() || before.Mode()&os.ModeSymlink != 0 {
		return "", fmt.Errorf("allocation directory unavailable")
	}
	f, err := os.Open(dir)
	if err != nil {
		return "", err
	}
	defer f.Close()
	after, err := f.Stat()
	if err != nil || !os.SameFile(before, after) {
		return "", fmt.Errorf("allocation changed")
	}
	var info windows.ByHandleFileInformation
	if err := windows.GetFileInformationByHandle(windows.Handle(f.Fd()), &info); err != nil {
		return "", err
	}
	if info.FileAttributes&windows.FILE_ATTRIBUTE_REPARSE_POINT != 0 {
		return "", fmt.Errorf("reparse allocation refused")
	}
	return fmt.Sprintf("windows:%x:%x:%x", info.VolumeSerialNumber, info.FileIndexHigh, info.FileIndexLow), nil
}

func singleLink(f *os.File, _ os.FileInfo) bool {
	var info windows.ByHandleFileInformation
	return windows.GetFileInformationByHandle(windows.Handle(f.Fd()), &info) == nil && info.NumberOfLinks == 1 && info.FileAttributes&windows.FILE_ATTRIBUTE_REPARSE_POINT == 0
}
