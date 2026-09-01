//go:build !windows

package main

import "strings"

// Linux and the other non-Windows hosts preserve the byte-exact containment
// comparison. Case-folding this allow-list outside Windows would grant a second,
// distinct path on a case-sensitive filesystem.
func canonicalPathsEqual(a, b string) bool { return a == b }

func canonicalPathHasPrefix(path, prefix string) bool { return strings.HasPrefix(path, prefix) }
