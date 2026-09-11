package taskartifacts

import (
	"golang.org/x/sys/windows"
	"os"
	"path/filepath"
	"testing"
)

func TestWindowsPrivateCustodyUsesNativeACLAndIdentity(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "custody")
	if err := MakePrivateDirectory(dir); err != nil {
		t.Fatal(err)
	}
	before, err := DirectoryIdentity(dir)
	if err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(dir, "report.md")
	if err := os.WriteFile(file, []byte("report\r\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := VerifyPrivateTree(dir); err != nil {
		t.Fatal(err)
	}
	sd, err := windows.SecurityDescriptorFromString("D:P(A;;FA;;;WD)")
	if err != nil {
		t.Fatal(err)
	}
	acl, _, err := sd.DACL()
	if err != nil {
		t.Fatal(err)
	}
	if err := windows.SetNamedSecurityInfo(file, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION, nil, nil, acl, nil); err != nil {
		t.Fatal(err)
	}
	if VerifyPrivateTree(dir) == nil {
		t.Fatal("world-readable artifact accepted despite synthetic mode 0600")
	}
	if err := os.Rename(dir, dir+"-retained"); err != nil {
		t.Fatal(err)
	}
	if err := MakePrivateDirectory(dir); err != nil {
		t.Fatal(err)
	}
	after, err := DirectoryIdentity(dir)
	if err != nil {
		t.Fatal(err)
	}
	if before == after {
		t.Fatal("replacement allocation reused native identity")
	}
}
