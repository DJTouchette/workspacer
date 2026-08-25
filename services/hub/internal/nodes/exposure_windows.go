//go:build windows

package nodes

import (
	"fmt"
	"unsafe"

	"golang.org/x/sys/windows"
)

// worldSIDs are the well-known principals that mean "somebody other than the
// account that owns this file". A read grant to any of them on a file holding
// a Fly token is the thing the warning exists to catch.
//
// Deliberately a fixed list of well-known SIDs rather than an attempt at a
// general answer: a DOMAIN group (S-1-5-21-…-513 "Domain Users") is equally
// "beyond the owner" and cannot be recognised from its SID alone. Those land
// in [ExposureUnknown] below — not silently in "fine".
var worldSIDs = map[string]string{
	"S-1-1-0":      "Everyone",
	"S-1-2-0":      "LOCAL",
	"S-1-5-2":      "NETWORK",
	"S-1-5-4":      "INTERACTIVE",
	"S-1-5-7":      "ANONYMOUS LOGON",
	"S-1-5-11":     "Authenticated Users",
	"S-1-5-113":    "Local account",
	"S-1-5-32-545": "BUILTIN\\Users",
	"S-1-5-32-546": "BUILTIN\\Guests",
	"S-1-5-32-547": "BUILTIN\\Power Users",
}

// benignSIDs can already read every file on the machine, so their presence in
// a DACL is not a leak and warning about it would be noise. Administrators is
// on this list for the same reason root is not a Unix exposure: an attacker
// who is SYSTEM or an admin does not need the ACL's permission.
//
// S-1-5-114 ("Local account and member of Administrators group") is
// deliberately here and not in worldSIDs: membership of it implies
// Administrators, which is already benign.
var benignSIDs = map[string]string{
	"S-1-5-18":     "SYSTEM",
	"S-1-5-19":     "LOCAL SERVICE",
	"S-1-5-20":     "NETWORK SERVICE",
	"S-1-5-32-544": "BUILTIN\\Administrators",
	"S-1-5-114":    "Local account and member of Administrators group",
	"S-1-3-0":      "CREATOR OWNER",
	"S-1-3-4":      "OWNER RIGHTS",
}

// readMask is every access right that lets a principal see the token. Note
// FILE_GENERIC_READ (0x120089) contains FILE_READ_DATA, so it is covered.
const readMask = windows.FILE_READ_DATA | windows.GENERIC_READ | windows.GENERIC_ALL

// fileExposure reads the file's real DACL, because on Windows the file mode Go
// reports is a synthetic 0666/0444 that describes one attribute bit and not a
// single thing about who can open the file. See exposure.go.
//
// The rule, in order of what it can establish:
//
//   - a read grant to a well-known everyone-ish SID  → [ExposureLoose]
//   - only the owner, this process's own account, or
//     SYSTEM/Administrators                          → [ExposureOwnerOnly]
//   - anything else in the DACL, or a DACL that
//     cannot be read                                 → [ExposureUnknown]
//
// The third case is the one this design is for. A domain group, a custom local
// group, an unusual ACE type: the honest answer is that the hub does not know,
// and it says so rather than certifying the file.
func fileExposure(path string) (Exposure, string) {
	sd, err := windows.GetNamedSecurityInfo(path, windows.SE_FILE_OBJECT,
		windows.OWNER_SECURITY_INFORMATION|windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		return ExposureUnknown, fmt.Sprintf("its security descriptor could not be read (%v)", err)
	}
	owner, _, err := sd.Owner()
	if err != nil {
		return ExposureUnknown, fmt.Sprintf("its owner could not be read (%v)", err)
	}
	dacl, _, err := sd.DACL()
	if err != nil {
		return ExposureUnknown, fmt.Sprintf("its DACL could not be read (%v)", err)
	}
	if dacl == nil {
		// Present but NULL. This is not "no permissions", it is the maximally
		// permissive case: a NULL DACL grants everyone full control.
		return ExposureLoose, "it has a NULL DACL, which grants EVERYONE full control"
	}
	if dacl.AceCount == 0 {
		// An empty DACL grants nobody anything — the opposite of NULL.
		return ExposureOwnerOnly, ""
	}

	ownerSID := ""
	if owner != nil {
		ownerSID = owner.String()
	}
	selfSID := currentUserSID()

	// Canonical ACL order puts DENY aces before ALLOW aces, so a deny seen
	// earlier in the walk correctly suppresses a later allow for the same SID.
	// A non-canonical ACL can only make this warn where Windows would not,
	// which is the safe direction for a security notice.
	denied := map[string]bool{}
	var unknown string

	for i := uint32(0); i < uint32(dacl.AceCount); i++ {
		var ace *windows.ACCESS_ALLOWED_ACE
		if err := windows.GetAce(dacl, i, &ace); err != nil {
			return ExposureUnknown, fmt.Sprintf("ACE %d of its DACL could not be read (%v)", i, err)
		}
		// INHERIT_ONLY aces apply to children, not to this object. A file has
		// no children, so such an ace grants nothing here.
		if ace.Header.AceFlags&windows.INHERIT_ONLY_ACE != 0 {
			continue
		}
		if ace.Header.AceType != windows.ACCESS_ALLOWED_ACE_TYPE &&
			ace.Header.AceType != windows.ACCESS_DENIED_ACE_TYPE {
			// Object and callback aces put the SID at a different offset, so
			// SidStart would be misread. Refuse to guess.
			unknown = fmt.Sprintf("its DACL contains an ACE of type %d that the hub cannot read", ace.Header.AceType)
			continue
		}
		// Safe for both allowed and denied aces: ACCESS_DENIED_ACE has the
		// identical Header/Mask/SidStart layout.
		sid := (*windows.SID)(unsafe.Pointer(&ace.SidStart))
		// SID.String() is "" when the SID cannot be converted. Guard it, or
		// an unreadable SID would compare equal to an unreadable owner and be
		// waved through as benign.
		s := sid.String()
		reads := ace.Mask&readMask != 0
		if s == "" {
			unknown = fmt.Sprintf("ACE %d of its DACL names a principal the hub could not read", i)
			continue
		}
		if ace.Header.AceType == windows.ACCESS_DENIED_ACE_TYPE {
			if reads {
				denied[s] = true
			}
			continue
		}
		if !reads {
			continue
		}
		if who, ok := worldSIDs[s]; ok {
			if denied[s] {
				continue
			}
			return ExposureLoose, fmt.Sprintf("its ACL grants read to %s (%s) — remove that entry (Properties → Security, or `icacls %q /remove:g *%s`)", who, s, path, s)
		}
		if (ownerSID != "" && s == ownerSID) || (selfSID != "" && s == selfSID) || benignSIDs[s] != "" {
			continue
		}
		unknown = fmt.Sprintf("its ACL grants read to %s, which the hub cannot classify as the owner, SYSTEM or Administrators", s)
	}
	if unknown != "" {
		return ExposureUnknown, unknown
	}
	return ExposureOwnerOnly, ""
}

// currentUserSID is the account the hub itself runs as. Its read access is
// what makes the file usable, not what makes it exposed — and it is not always
// the owner (a file created by a member of Administrators can be owned by the
// Administrators group instead of by the creator).
//
// "" when the token cannot be read, which simply means one fewer principal is
// recognised and the answer degrades to [ExposureUnknown] rather than to a
// false alarm.
func currentUserSID() string {
	u, err := windows.GetCurrentProcessToken().GetTokenUser()
	if err != nil || u == nil || u.User.Sid == nil {
		return ""
	}
	return u.User.Sid.String()
}
