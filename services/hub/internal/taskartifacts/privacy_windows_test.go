package taskartifacts

import (
	"context"
	"golang.org/x/sys/windows"
	"os"
	"path/filepath"
	"strings"
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

func TestWindowsCRLFSourceUsesCommittedCheckpointSemantics(t *testing.T) {
	ctx := context.Background()
	repo := t.TempDir()
	git := func(args ...string) {
		t.Helper()
		if _, err := Git(ctx, repo, "", args...); err != nil {
			t.Fatal(err)
		}
	}
	git("init")
	git("config", "--local", "core.autocrlf", "true")
	file := filepath.Join(repo, "code.txt")
	if err := os.WriteFile(file, []byte("line one\nline two\n"), 0600); err != nil {
		t.Fatal(err)
	}
	git("add", "code.txt")
	git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "commit", "-m", "LF checkpoint")
	if err := os.Remove(file); err != nil {
		t.Fatal(err)
	}
	git("-c", "core.autocrlf=true", "checkout", "--", "code.txt")
	data, err := os.ReadFile(file)
	if err != nil || !strings.Contains(string(data), "\r\n") {
		t.Fatal("fixture did not produce Windows CRLF checkout", err)
	}
	if _, _, err := CheckSource(ctx, repo); err != nil {
		t.Fatal("ordinary CRLF source refused", err)
	}
	if err := os.WriteFile(file, []byte("actual uncommitted edit\r\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, _, err := CheckSource(ctx, repo); err == nil {
		t.Fatal("tracked WIP was hidden")
	}
}
