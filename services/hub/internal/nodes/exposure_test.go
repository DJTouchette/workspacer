package nodes

import (
	"path/filepath"
	"testing"
)

// The registry holds a credential that spends money. A caller must be able to
// notice it is readable beyond its owner — and must never be told it is fine
// on a platform where the hub cannot tell.
//
// This file holds the claims that are true on EVERY platform. The
// platform-specific mechanics (mode bits / ACLs) are in exposure_unix_test.go
// and exposure_windows_test.go, because the two have nothing in common except
// the contract asserted here.

// The assertion that went red on containment-windows, kept as the
// cross-platform claim it always was: a file the process just created for
// itself is not "readable by everyone", and any check that says otherwise is
// not reading the right thing. The old boolean returned true here on Windows
// for every file that existed. See exposure.go.
func TestAFreshOwnerOnlyFileIsNeverReportedLoose(t *testing.T) {
	p := write(t, `[]`)
	if got, why := FileExposure(p); got == ExposureLoose {
		t.Fatalf("a file this process just created 0600 was reported %v: %s", got, why)
	}
}

// A path that cannot be examined is UNKNOWN, not fine. The boolean this
// replaces answered `false` — indistinguishable from "checked, owner-only".
func TestAnUnexaminableFileIsUnknownAndNotFine(t *testing.T) {
	got, why := FileExposure(filepath.Join(t.TempDir(), "absent.json"))
	if got != ExposureUnknown {
		t.Fatalf("FileExposure(absent) = %v, want %v", got, ExposureUnknown)
	}
	if why == "" {
		t.Error("an unknown answer came with no explanation — the caller has nothing to print")
	}
}

// The zero value is the ignorant one on purpose: a switch that forgets a case,
// or a struct field nobody set, must not read as a clean bill of health.
func TestTheZeroExposureIsUnknown(t *testing.T) {
	var e Exposure
	if e != ExposureUnknown {
		t.Fatalf("zero Exposure = %v, want unknown — a forgotten value must never mean safe", e)
	}
}

// Every answer that asks the user to go and look owes them a reason.
func TestOnlyOwnerOnlyMayAnswerWithoutAReason(t *testing.T) {
	p := write(t, `[]`)
	got, why := FileExposure(p)
	if got != ExposureOwnerOnly && why == "" {
		t.Fatalf("FileExposure = %v with no explanation", got)
	}
	if got == ExposureOwnerOnly && why != "" {
		t.Fatalf("owner-only came with an explanation %q — nothing to explain", why)
	}
}
