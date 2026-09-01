//go:build windows

package routing

import (
	"fmt"
	"os"

	"golang.org/x/sys/windows"
)

func openDecisionLogFile(path string) (*os.File, error) {
	p, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return nil, err
	}
	// FILE_APPEND_DATA without FILE_WRITE_DATA gives the same append-only
	// guarantee as O_APPEND. WRITE_DAC is requested on this very handle so the
	// privacy repair below cannot race a path replacement.
	h, err := windows.CreateFile(p,
		windows.FILE_APPEND_DATA|windows.WRITE_DAC|windows.READ_CONTROL,
		windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE,
		nil, windows.OPEN_ALWAYS, windows.FILE_ATTRIBUTE_NORMAL, 0)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(h), path), nil
}

// secureDecisionLogFile replaces inheritance with an owner-only DACL before a
// row is written. Go's 0600 mode on Windows only toggles the read-only file
// attribute; it grants and revokes no principal.
func secureDecisionLogFile(f *os.File) error {
	user, err := windows.GetCurrentProcessToken().GetTokenUser()
	if err != nil {
		return fmt.Errorf("read current user SID: %w", err)
	}
	if user == nil || user.User.Sid == nil {
		return fmt.Errorf("read current user SID: token returned no user")
	}
	system, err := windows.CreateWellKnownSid(windows.WinLocalSystemSid)
	if err != nil {
		return fmt.Errorf("create SYSTEM SID: %w", err)
	}
	admins, err := windows.CreateWellKnownSid(windows.WinBuiltinAdministratorsSid)
	if err != nil {
		return fmt.Errorf("create Administrators SID: %w", err)
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
		return fmt.Errorf("build owner-only DACL: %w", err)
	}
	return windows.SetSecurityInfo(windows.Handle(f.Fd()), windows.SE_FILE_OBJECT,
		windows.DACL_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION,
		nil, nil, dacl, nil)
}
