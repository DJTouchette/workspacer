//go:build windows

package routing

// Windows resolves drive letters, UNC hosts/shares, and ordinary NTFS path
// components case-insensitively. A ceiling must answer for the directory the
// filesystem opens, not for the caller's or routing.yaml's casing of it.
func routingPathsEqual(a, b string) bool { return windowsRoutingPathsEqual(a, b) }

func routingPathHasPrefix(path, prefix string) bool { return windowsRoutingPathHasPrefix(path, prefix) }
