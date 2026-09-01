package routing

import "strings"

// Kept host-independent so the Windows comparison primitive can be mutation-
// tested on every developer and CI host; matrix_windows_test.go drives it
// through filepath's real Windows volume and separator implementation.
func windowsRoutingPathsEqual(a, b string) bool { return strings.EqualFold(a, b) }

func windowsRoutingPathHasPrefix(path, prefix string) bool {
	return len(path) >= len(prefix) && strings.EqualFold(path[:len(prefix)], prefix)
}
