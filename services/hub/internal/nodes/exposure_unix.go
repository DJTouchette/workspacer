//go:build !windows

package nodes

import (
	"fmt"
	"os"
)

// fileExposure on a platform whose file mode really is the permission set.
// The group and other read/write/execute bits are the whole answer here, so
// this half is allowed to be as short as it looks — see exposure.go for why
// its Windows twin is not.
//
// os.Stat, not os.Lstat: what matters is who can read the BYTES, so a symlink
// is followed to the file that actually holds the token.
func fileExposure(path string) (Exposure, string) {
	st, err := os.Stat(path)
	if err != nil {
		// The old boolean answered "not exposed" here, which reads as a clean
		// bill of health for a file nobody could even stat.
		return ExposureUnknown, fmt.Sprintf("it could not be examined (%v)", err)
	}
	if perm := st.Mode().Perm(); perm&0o077 != 0 {
		return ExposureLoose, fmt.Sprintf("its mode is %#o — group or other can reach it; chmod 600 it", perm)
	}
	return ExposureOwnerOnly, ""
}
