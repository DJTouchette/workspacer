//go:build !windows

package nodes

import (
	"os"
	"strings"
	"testing"
)

// On a platform where the mode bits ARE the permission set, the check is
// exact in both directions, and the test can say so in both directions.
func TestFileExposureReadsTheModeBits(t *testing.T) {
	cases := []struct {
		mode os.FileMode
		want Exposure
	}{
		{0o600, ExposureOwnerOnly},
		{0o400, ExposureOwnerOnly},
		{0o644, ExposureLoose}, // other can read
		{0o640, ExposureLoose}, // group can read
		{0o604, ExposureLoose}, // other can read, group cannot
		{0o601, ExposureLoose}, // execute is still a bit somebody else holds
	}
	p := write(t, `[]`)
	for _, c := range cases {
		if err := os.Chmod(p, c.mode); err != nil {
			t.Fatal(err)
		}
		got, why := FileExposure(p)
		if got != c.want {
			t.Errorf("FileExposure(%#o) = %v (%s), want %v", c.mode, got, why, c.want)
		}
		if c.want == ExposureLoose && !strings.Contains(why, "chmod 600") {
			t.Errorf("the %#o warning does not tell the user how to fix it: %q", c.mode, why)
		}
	}
}
