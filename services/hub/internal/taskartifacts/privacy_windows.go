package taskartifacts

import (
	"fmt"
	"os"
	"path/filepath"
	"unsafe"

	"golang.org/x/sys/windows"
)

// Windows modes are synthetic attribute bits, not access permissions. Install
// an inheritable owner/SYSTEM/Administrators DACL on a NEW task directory, then
// inspect the actual DACL. Existing directories are checked, never re-permissioned.
func MakePrivateDirectory(dir string) error {
	user, err := windows.GetCurrentProcessToken().GetTokenUser()
	if err != nil {
		return fmt.Errorf("private task storage account could not be resolved")
	}
	sid := user.User.Sid.String()
	if sid == "" {
		return fmt.Errorf("private task storage account is unavailable")
	}
	if err := os.MkdirAll(filepath.Dir(dir), 0700); err != nil {
		return err
	}
	if err := os.Mkdir(dir, 0700); err == nil {
		sd, err := windows.SecurityDescriptorFromString("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;" + sid + ")")
		if err != nil {
			return err
		}
		acl, _, err := sd.DACL()
		if err != nil {
			return err
		}
		if err := windows.SetNamedSecurityInfo(dir, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION, nil, nil, acl, nil); err != nil {
			return fmt.Errorf("private task storage ACL could not be installed")
		}
	} else if !os.IsExist(err) {
		return err
	}
	info, err := os.Lstat(dir)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return fmt.Errorf("task storage must be a real directory")
	}
	sd, err := windows.GetNamedSecurityInfo(dir, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		return fmt.Errorf("private task storage ACL could not be read")
	}
	acl, _, err := sd.DACL()
	if err != nil || acl == nil {
		return fmt.Errorf("private task storage has an unknown or unrestricted ACL")
	}
	for i := uint32(0); i < uint32(acl.AceCount); i++ {
		var ace *windows.ACCESS_ALLOWED_ACE
		if err := windows.GetAce(acl, i, &ace); err != nil {
			return fmt.Errorf("private task storage ACL could not be verified")
		}
		if ace.Header.AceType == windows.ACCESS_DENIED_ACE_TYPE {
			continue
		}
		if ace.Header.AceType != windows.ACCESS_ALLOWED_ACE_TYPE {
			return fmt.Errorf("private task storage ACL type is unsupported")
		}
		principal := (*windows.SID)(unsafe.Pointer(&ace.SidStart)).String()
		if principal != sid && principal != "S-1-5-18" && principal != "S-1-5-32-544" && principal != "S-1-3-0" {
			return fmt.Errorf("task storage ACL grants access beyond the account and machine administrators")
		}
	}
	return nil
}
