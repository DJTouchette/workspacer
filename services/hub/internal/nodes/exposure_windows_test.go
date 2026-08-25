//go:build windows

package nodes

import (
	"os"
	"strings"
	"testing"

	"golang.org/x/sys/windows"
)

// The two halves of the Windows story, each pinned by a test, because neither
// is guessable from the Unix code and both were got wrong once.

// os.Chmod on Windows toggles FILE_ATTRIBUTE_READONLY and NOTHING ELSE
// (syscall/syscall_windows.go). It grants no principal any access, so a file
// this process created for itself is still owner-only after `chmod 0644` —
// and a check that reports it exposed is reading the wrong thing.
//
// This is the exact inverse of the assertion that used to be here and passed
// on Windows for the wrong reason: back then EVERY file read as exposed, so
// the 0644 half was vacuous and the 0600 half was the one that went red.
func TestChmodDoesNotExposeAFileOnWindows(t *testing.T) {
	p := write(t, `[]`)
	before, err := os.Stat(p)
	if err != nil {
		t.Fatal(err)
	}
	for _, mode := range []os.FileMode{0o644, 0o666, 0o400} {
		if err := os.Chmod(p, mode); err != nil {
			t.Fatal(err)
		}
		if got, why := FileExposure(p); got == ExposureLoose {
			t.Errorf("chmod %#o reported the file exposed (%s) — chmod grants nobody anything on Windows", mode, why)
		}
	}
	// And the reason a mode-bit check cannot work here: Go synthesises Perm()
	// from the read-only attribute alone, so it moves for a chmod that changed
	// no permission and would not move for an ACL change that changed a real
	// one. 0666 or 0444, never anything else — and `&0o077` is set in both.
	if err := os.Chmod(p, 0o600); err != nil {
		t.Fatal(err)
	}
	perm := before.Mode().Perm()
	if perm != 0o666 && perm != 0o444 {
		t.Fatalf("Perm() = %#o; Windows file modes were expected to be synthetic 0666/0444", perm)
	}
	if perm&0o077 == 0 {
		t.Fatal("Perm()&0o077 was clear — if Windows ever grows real mode bits, the Unix check could be shared")
	}
}

// The half that matters: a file whose ACL really does grant Everyone read IS
// reported loose. Without this the Windows implementation would be a function
// that compiles and always says "owner-only", which is the failure this whole
// change exists to avoid.
func TestAnEveryoneReadAceIsReportedLoose(t *testing.T) {
	p := write(t, `[]`)
	if got, why := FileExposure(p); got == ExposureLoose {
		t.Fatalf("the file was already loose before the test touched its ACL (%s)", why)
	}

	grantEveryoneRead(t, p)

	got, why := FileExposure(p)
	if got != ExposureLoose {
		t.Fatalf("FileExposure after granting Everyone read = %v (%s), want loose", got, why)
	}
	if !strings.Contains(why, "Everyone") {
		t.Errorf("the warning does not name the principal that can read the token: %q", why)
	}
}

// grantEveryoneRead adds an Everyone:(R) entry to the file's existing DACL —
// `icacls <file> /grant Everyone:(R)`, via the API so the test needs no shell.
func grantEveryoneRead(t *testing.T, path string) {
	t.Helper()
	sd, err := windows.GetNamedSecurityInfo(path, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		t.Fatalf("GetNamedSecurityInfo: %v", err)
	}
	dacl, _, err := sd.DACL()
	if err != nil {
		t.Fatalf("DACL: %v", err)
	}
	everyone, err := windows.CreateWellKnownSid(windows.WinWorldSid)
	if err != nil {
		t.Fatalf("CreateWellKnownSid(Everyone): %v", err)
	}
	merged, err := windows.ACLFromEntries([]windows.EXPLICIT_ACCESS{{
		AccessPermissions: windows.GENERIC_READ,
		AccessMode:        windows.GRANT_ACCESS,
		Inheritance:       windows.NO_INHERITANCE,
		Trustee: windows.TRUSTEE{
			TrusteeForm:  windows.TRUSTEE_IS_SID,
			TrusteeType:  windows.TRUSTEE_IS_WELL_KNOWN_GROUP,
			TrusteeValue: windows.TrusteeValueFromSID(everyone),
		},
	}}, dacl)
	if err != nil {
		t.Fatalf("ACLFromEntries: %v", err)
	}
	if err := windows.SetNamedSecurityInfo(path, windows.SE_FILE_OBJECT,
		windows.DACL_SECURITY_INFORMATION, nil, nil, merged, nil); err != nil {
		t.Fatalf("SetNamedSecurityInfo: %v", err)
	}
}
