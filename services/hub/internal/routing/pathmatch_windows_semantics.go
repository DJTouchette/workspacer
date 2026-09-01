package routing

// Kept host-independent so the Windows comparison primitive can be mutation-
// tested on every developer and CI host; matrix_windows_test.go drives it
// through filepath's real Windows volume and separator implementation.
func windowsRoutingPathsEqual(a, b string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := 0; i < len(a); i++ {
		ac, bc := a[i], b[i]
		if ac >= 'A' && ac <= 'Z' {
			ac += 'a' - 'A'
		}
		if bc >= 'A' && bc <= 'Z' {
			bc += 'a' - 'A'
		}
		if ac != bc {
			return false
		}
	}
	return true
}

func windowsRoutingPathHasPrefix(path, prefix string) bool {
	return len(path) >= len(prefix) && windowsRoutingPathsEqual(path[:len(prefix)], prefix)
}
