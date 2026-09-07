//go:build windows

package routing

import (
	"golang.org/x/sys/windows"
	"os"
	"path/filepath"
	"runtime"
	"unsafe"
)

// Create the temporary sidecar with a protected DACL before writing any data;
// chmod(0600) alone does not restrict readers on Windows. Reuse the log's
// owner/SYSTEM/Administrators policy, but request ordinary write access.
func createPreferencesTemp(dir string) (*os.File, error) {
	name := filepath.Join(dir, ".routing-preferences-"+NewDecisionID())
	p, err := windows.UTF16PtrFromString(name)
	if err != nil {
		return nil, err
	}
	sd, err := privateDecisionLogSecurityDescriptor()
	if err != nil {
		return nil, err
	}
	h, err := windows.CreateFile(p, windows.GENERIC_WRITE|windows.READ_CONTROL, windows.FILE_SHARE_READ,
		&windows.SecurityAttributes{Length: uint32(unsafe.Sizeof(windows.SecurityAttributes{})), SecurityDescriptor: sd}, windows.CREATE_NEW, windows.FILE_ATTRIBUTE_NORMAL, 0)
	runtime.KeepAlive(sd)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(h), name), nil
}
