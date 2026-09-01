//go:build windows

package routing

import (
	"fmt"
	"os"
	"runtime"
	"unsafe"

	"golang.org/x/sys/windows"
)

func openDecisionLogFile(path string) (*os.File, error) {
	p, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return nil, err
	}
	securityDescriptor, err := privateDecisionLogSecurityDescriptor()
	if err != nil {
		return nil, fmt.Errorf("build private creation security descriptor: %w", err)
	}
	// FILE_APPEND_DATA without FILE_WRITE_DATA gives the same append-only
	// guarantee as O_APPEND. The protected DACL is supplied to CreateFile so a
	// NEW log is never visible through inherited permissions, even briefly.
	// Windows ignores that descriptor when OPEN_ALWAYS finds an existing file;
	// WRITE_DAC remains necessary so secureDecisionLogFile can repair that file
	// through this same handle without racing a path replacement.
	h, err := windows.CreateFile(p,
		windows.FILE_APPEND_DATA|windows.WRITE_DAC|windows.READ_CONTROL,
		windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE,
		&windows.SecurityAttributes{
			Length:             uint32(unsafe.Sizeof(windows.SecurityAttributes{})),
			SecurityDescriptor: securityDescriptor,
		}, windows.OPEN_ALWAYS, windows.FILE_ATTRIBUTE_NORMAL, 0)
	runtime.KeepAlive(securityDescriptor)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(h), path), nil
}

// secureDecisionLogFile replaces inheritance with an owner-only DACL before a
// row is written. Go's 0600 mode on Windows only toggles the read-only file
// attribute; it grants and revokes no principal.
func secureDecisionLogFile(f *os.File) error {
	_, dacl, err := privateDecisionLogDACL()
	if err != nil {
		return err
	}
	return windows.SetSecurityInfo(windows.Handle(f.Fd()), windows.SE_FILE_OBJECT,
		windows.DACL_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION,
		nil, nil, dacl, nil)
}

func privateDecisionLogSecurityDescriptor() (*windows.SECURITY_DESCRIPTOR, error) {
	owner, dacl, err := privateDecisionLogDACL()
	if err != nil {
		return nil, err
	}
	absolute, err := windows.NewSecurityDescriptor()
	if err != nil {
		return nil, fmt.Errorf("initialize security descriptor: %w", err)
	}
	if err := absolute.SetOwner(owner, false); err != nil {
		return nil, fmt.Errorf("set decision-log owner: %w", err)
	}
	if err := absolute.SetDACL(dacl, true, false); err != nil {
		return nil, fmt.Errorf("set decision-log DACL: %w", err)
	}
	if err := absolute.SetControl(windows.SE_DACL_PROTECTED, windows.SE_DACL_PROTECTED); err != nil {
		return nil, fmt.Errorf("protect decision-log DACL: %w", err)
	}
	selfRelative, err := absolute.ToSelfRelative()
	if err != nil {
		return nil, fmt.Errorf("make decision-log security descriptor self-relative: %w", err)
	}
	return selfRelative, nil
}

func privateDecisionLogDACL() (*windows.SID, *windows.ACL, error) {
	user, err := windows.GetCurrentProcessToken().GetTokenUser()
	if err != nil {
		return nil, nil, fmt.Errorf("read current user SID: %w", err)
	}
	if user == nil || user.User.Sid == nil {
		return nil, nil, fmt.Errorf("read current user SID: token returned no user")
	}
	system, err := windows.CreateWellKnownSid(windows.WinLocalSystemSid)
	if err != nil {
		return nil, nil, fmt.Errorf("create SYSTEM SID: %w", err)
	}
	admins, err := windows.CreateWellKnownSid(windows.WinBuiltinAdministratorsSid)
	if err != nil {
		return nil, nil, fmt.Errorf("create Administrators SID: %w", err)
	}

	entries := make([]windows.EXPLICIT_ACCESS, 0, 3)
	for _, sid := range []*windows.SID{user.User.Sid, system, admins} {
		entries = append(entries, windows.EXPLICIT_ACCESS{
			AccessPermissions: windows.GENERIC_ALL,
			AccessMode:        windows.SET_ACCESS,
			Inheritance:       windows.NO_INHERITANCE,
			Trustee: windows.TRUSTEE{
				TrusteeForm:  windows.TRUSTEE_IS_SID,
				TrusteeType:  windows.TRUSTEE_IS_UNKNOWN,
				TrusteeValue: windows.TrusteeValueFromSID(sid),
			},
		})
	}
	dacl, err := windows.ACLFromEntries(entries, nil)
	if err != nil {
		return nil, nil, fmt.Errorf("build private DACL: %w", err)
	}
	return user.User.Sid, dacl, nil
}
