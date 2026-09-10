package taskartifacts

import (
	"golang.org/x/sys/windows"
	"os"
)

func singleLink(f *os.File, _ os.FileInfo) bool {
	var info windows.ByHandleFileInformation
	return windows.GetFileInformationByHandle(windows.Handle(f.Fd()), &info) == nil && info.NumberOfLinks == 1
}
