//go:build windows

package routing

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/nodes"
	"golang.org/x/sys/windows"
)

// Go exposes only the Windows read-only attribute as synthetic 0666/0444 mode
// bits. The confidentiality oracle is the ACL abstraction, not Perm().
func TestDecisionLogPrivacyOnWindowsIsAnACLContract(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	NewDecisionLog(path, 0).Decision(Decision{DecisionID: "rd_x"})
	st, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if perm := st.Mode().Perm(); perm != 0o666 && perm != 0o444 {
		t.Fatalf("Windows mode = %v, want the documented synthetic 0666/0444", perm)
	}
	if got, why := nodes.FileExposure(path); got != nodes.ExposureOwnerOnly {
		t.Fatalf("decision log exposure = %v (%s), want owner-only", got, why)
	}
	sd, err := windows.GetNamedSecurityInfo(path, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		t.Fatalf("GetNamedSecurityInfo: %v", err)
	}
	control, _, err := sd.Control()
	if err != nil {
		t.Fatalf("security descriptor control: %v", err)
	}
	if control&windows.SE_DACL_PROTECTED == 0 {
		t.Fatal("decision log DACL still inherits from its parent")
	}
}

func TestOpenDecisionLogFileCreatesWithProtectedDACLBeforeRepair(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "loose-parent")
	if err := os.Mkdir(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	grantDecisionLogEveryoneReadWithInheritance(t, dir, windows.SUB_CONTAINERS_AND_OBJECTS_INHERIT)
	if got, why := nodes.FileExposure(dir); got != nodes.ExposureLoose {
		t.Fatalf("parent ACL mutation did not make the atomic-creation guard red: exposure=%v (%s)", got, why)
	}

	path := filepath.Join(dir, "routing-decisions.jsonl")
	f, err := openDecisionLogFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	// Deliberately do not call secureDecisionLogFile: this assertion observes
	// the ACL installed by CreateFile itself, before the existing-file repair.
	if got, why := nodes.FileExposure(path); got != nodes.ExposureOwnerOnly {
		t.Fatalf("new decision log exposure before repair = %v (%s), want owner-only", got, why)
	}
	sd, err := windows.GetNamedSecurityInfo(path, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		t.Fatalf("GetNamedSecurityInfo: %v", err)
	}
	control, _, err := sd.Control()
	if err != nil {
		t.Fatalf("security descriptor control: %v", err)
	}
	if control&windows.SE_DACL_PROTECTED == 0 {
		t.Fatal("CreateFile did not protect the decision-log DACL from inheritance")
	}
}

// Reversible mutation guard: make an existing log genuinely loose at the ACL
// layer, prove the oracle sees it, then append and require the writer to repair
// privacy before the sensitive row lands.
func TestDecisionLogRepairsALooseWindowsDACLBeforeAppend(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	if err := os.WriteFile(path, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	grantDecisionLogEveryoneRead(t, path)
	if got, why := nodes.FileExposure(path); got != nodes.ExposureLoose {
		t.Fatalf("ACL mutation did not make the guard red: exposure=%v (%s)", got, why)
	}

	NewDecisionLog(path, 0).Decision(Decision{DecisionID: "rd_repaired"})
	if got, why := nodes.FileExposure(path); got != nodes.ExposureOwnerOnly {
		t.Fatalf("append did not repair the loose ACL: exposure=%v (%s)", got, why)
	}
	if entries := readEntries(t, path); len(entries) != 1 || entries[0].DecisionID != "rd_repaired" {
		t.Fatalf("repaired log entries = %+v", entries)
	}
}

func TestDecisionLogRepairsALooseWindowsDACLBeforeRotation(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	if err := os.WriteFile(path, []byte("loose oversized generation\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	grantDecisionLogEveryoneRead(t, path)
	if got, why := nodes.FileExposure(path); got != nodes.ExposureLoose {
		t.Fatalf("ACL mutation did not make the rotation guard red: exposure=%v (%s)", got, why)
	}

	NewDecisionLog(path, 1).Decision(Decision{DecisionID: "rd_rotated_private"})
	rotated := path + ".1"
	if got, why := nodes.FileExposure(rotated); got != nodes.ExposureOwnerOnly {
		t.Fatalf("rotated generation exposure = %v (%s), want owner-only", got, why)
	}
	if entries := readEntries(t, path); len(entries) != 1 || entries[0].DecisionID != "rd_rotated_private" {
		t.Fatalf("new live generation entries = %+v", entries)
	}
}

func grantDecisionLogEveryoneRead(t *testing.T, path string) {
	t.Helper()
	grantDecisionLogEveryoneReadWithInheritance(t, path, windows.NO_INHERITANCE)
}

func grantDecisionLogEveryoneReadWithInheritance(t *testing.T, path string, inheritance uint32) {
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
		Inheritance:       inheritance,
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
